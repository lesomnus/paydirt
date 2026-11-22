//! Audited protobuf documents use the same patch wire format as Apply.
use crate::{Fault, Result, Store, value};
use buffa_reflect::{DescriptorPool, DynamicMessage, MapKey, Value as Proto};
use paydirt_model::Entity;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Content {
    Fields,
    #[default]
    Patch,
    Value,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub default: Content,
    pub by: BTreeMap<String, Content>,
    pub access: bool,
    pub downloads: BTreeSet<String>,
    pub reads: bool,
}
pub struct Recorder {
    pub(crate) keys: Option<crate::integrity::Keys>,
    pub(crate) options: Options,
    pub(crate) pool: DescriptorPool,
    pub(crate) sensitive: BTreeMap<String, BTreeSet<u32>>,
}

impl Store {
    pub fn with_trail(
        mut self,
        options: Options,
        descriptors: &[u8],
        schema: &str,
    ) -> Result<Self> {
        for name in options.by.keys() {
            self.model.entity(name).map_err(Fault::invalid)?;
        }
        let pool =
            DescriptorPool::decode(descriptors).map_err(|e| Fault::invalid(e.to_string()))?;
        let fields: BTreeMap<String, Value> =
            serde_json::from_str(schema).map_err(|e| Fault::invalid(e.to_string()))?;
        let sensitive = fields
            .into_iter()
            .map(|(k, v)| {
                let numbers = v["fields"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|f| f["personal"] == true || f["secret"] == true)
                    .filter_map(|f| f["number"].as_u64().map(|n| n as u32))
                    .collect();
                (k, numbers)
            })
            .collect();
        self.trail = Some(Recorder {
            keys: None,
            options,
            pool,
            sensitive,
        });
        Ok(self)
    }
}
fn key(k: &MapKey) -> Value {
    match k {
        MapKey::String(v) => json!({"s":v}),
        MapKey::Bool(v) => json!({"b":v}),
        MapKey::I32(v) => json!({"i":v.to_string()}),
        MapKey::U32(v) => json!({"u":v.to_string()}),
        MapKey::I64(v) => json!({"i":v.to_string()}),
        MapKey::U64(v) => json!({"u":v.to_string()}),
    }
}
fn literal(v: &Proto) -> Value {
    match v {
        Proto::Bool(v) => json!({"b":v}),
        Proto::I32(v) => json!({"i32":v}),
        Proto::U32(v) => json!({"u32":v}),
        Proto::I64(v) => json!({"i64":v.to_string()}),
        Proto::U64(v) => json!({"u64":v.to_string()}),
        Proto::F32(v) => json!({"f32":v}),
        Proto::F64(v) => json!({"f64":v}),
        Proto::String(v) => json!({"s":v}),
        Proto::Bytes(v) => json!({"x":value::encoded(v)}),
        Proto::EnumNumber(v) => json!({"e":v}),
        Proto::List(v) => json!({"l":{"values":v.iter().map(literal).collect::<Vec<_>>()}}),
        Proto::Message(v) => {
            json!({"m":{"fields":v.fields().map(|(f,v)|json!({"key":{"name":f.name(),"number":f.number()},"value":literal(v)})).collect::<Vec<_>>()}})
        }
        Proto::Map(map) => {
            let ordered: BTreeMap<_, _> = map.iter().collect();
            json!({"map":{"entries":ordered.into_iter().map(|(k,v)|json!({"key":key(k),"value":literal(v)})).collect::<Vec<_>>()}})
        }
    }
}
impl Recorder {
    pub(crate) fn message(&self, name: &str, doc: Value) -> Result<DynamicMessage> {
        let desc = self
            .pool
            .get_message_by_name(name)
            .ok_or_else(|| Fault::internal(format!("missing {name}")))?;
        DynamicMessage::deserialize(desc, doc).map_err(|e| Fault::internal(e.to_string()))
    }
    /// Clear annotations before snapshot or nested patch literals are encoded.
    fn redact(&self, msg: &mut DynamicMessage) {
        let desc = msg.descriptor().clone();
        for f in desc.fields() {
            if !msg.has_field(&f) {
                continue;
            }
            if self
                .sensitive
                .get(desc.full_name())
                .is_some_and(|s| s.contains(&f.number()))
            {
                msg.clear_field(&f);
                continue;
            }
            self.redact_value(msg.get_field_mut(&f));
        }
    }
    fn redact_value(&self, v: &mut Proto) {
        match v {
            Proto::Message(m) => self.redact(m),
            Proto::List(l) => l.iter_mut().for_each(|v| self.redact_value(v)),
            Proto::Map(m) => m.values_mut().for_each(|v| self.redact_value(v)),
            _ => {}
        }
    }
    pub(crate) fn content(
        &self,
        e: &Entity,
        request: &Value,
        before: Option<&Value>,
        after: Option<&Value>,
        audit: &mut Value,
    ) -> Result<()> {
        let level = self
            .options
            .by
            .get(&e.name)
            .copied()
            .unwrap_or(self.options.default);
        let fields: Vec<_> = e
            .fields
            .iter()
            .filter(|f| {
                if after.is_none() {
                    return false;
                }
                if before.is_none() {
                    return after
                        .and_then(|r| r.get(&f.json))
                        .is_some_and(|v| !v.is_null());
                }
                !f.version
                    && (request.get(&f.json).is_some()
                        || request.get(format!("{}Null", f.json)).is_some()
                        || before.and_then(|v| v.get(&f.json))
                            != after.and_then(|v| v.get(&f.json)))
            })
            .collect();
        audit["fields"] = json!(fields.iter().map(|f| f.number).collect::<BTreeSet<_>>());
        if level == Content::Fields || after.is_none() {
            return Ok(());
        }
        let mut msg = self.message(&e.name, after.unwrap().clone())?;
        self.redact(&mut msg);
        if level == Content::Value {
            audit["value"] = value::encoded(&msg.encode_to_vec());
        }
        let mut entries = Vec::new();
        for f in fields {
            if self
                .sensitive
                .get(&e.name)
                .is_some_and(|set| set.contains(&f.number))
            {
                continue;
            }
            let fd = msg
                .descriptor()
                .get_field_by_number(f.number)
                .ok_or_else(|| Fault::internal("missing field descriptor"))?;
            let mut entry = json!({"targets":{"selectors":[{"key":{"field":{"name":f.name,"number":f.number}}}]}});
            if after.unwrap()[&f.json].is_null() {
                entry["remove"] = json!({})
            } else {
                entry["assign"] = json!({"value":literal(&msg.get_field(&fd))})
            }
            entries.push(entry);
        }
        if !entries.is_empty() {
            let doc = json!({"messageType":e.name,"delta":{"entries":entries}});
            audit["patch"] = value::encoded(&self.message("patch.Patch", doc)?.encode_to_vec());
        }
        Ok(())
    }
}
