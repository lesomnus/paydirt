//! Protobuf resource changes retained in the committing transaction.
use crate::{Fault, Result, Store, Transaction, driver::Query, secrets, value};
use buffa_reflect::{DescriptorPool, DynamicMessage};
use paydirt_model::{Entity, quote};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
#[derive(Deserialize, Default, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub entities: BTreeSet<String>,
    pub previous: bool,
}
pub struct Recorder {
    options: Options,
    pool: DescriptorPool,
    secrets: secrets::Schema,
}
impl Store {
    pub fn with_resources(
        mut self,
        options: Options,
        descriptors: &[u8],
        fields: &str,
    ) -> Result<Self> {
        if options.entities.is_empty() {
            return Ok(self);
        }
        for n in &options.entities {
            let e = self.model.entity(n).map_err(Fault::invalid)?;
            if e.key().map_err(Fault::invalid)?.kind != "UUID"
                || self.policies.get(n).is_none_or(|p| {
                    !p.methods.contains_key("Get")
                        || p.entity.get("own").is_some_and(|k| {
                            !matches!(
                                k.as_str(),
                                Some("OWN_TENANT" | "OWN_HOLDER" | "OWN_EVENT" | "OWN_FACT")
                            )
                        })
                })
            {
                return Err(Fault::invalid(format!("cannot watch {n}")));
            }
        }
        self.resources = Some(Recorder {
            options,
            pool: DescriptorPool::decode(descriptors).map_err(|e| Fault::invalid(e.to_string()))?,
            secrets: secrets::Schema::new(fields)?,
        });
        Ok(self)
    }
}
impl Recorder {
    pub(crate) fn selected(&self, name: &str) -> bool {
        self.options.entities.contains(name)
    }
    pub(crate) fn record(
        &self,
        name: &str,
        method: &str,
        call: &str,
        before: Option<&Value>,
        after: Option<&Value>,
        id: Value,
        stamp: Value,
    ) -> Result<String> {
        let kind = match (before, after) {
            (None, Some(_)) => "CHANGE_KIND_CREATED",
            (Some(_), Some(_)) => "CHANGE_KIND_UPDATED",
            (Some(_), None) => "CHANGE_KIND_DELETED",
            _ => return Err(Fault::internal("empty resource change")),
        };
        let mut event =
            json!({"id":id,"method":method,"callMethod":call,"kind":kind,"dateCreated":stamp});
        let pack = |value: &Value| {
            let mut v = value.clone();
            v["@type"] = json!(format!("type.googleapis.com/{name}"));
            v
        };
        if let Some(v) = after {
            event["value"] = pack(v)
        }
        if let Some(v) = before.filter(|_| after.is_none() || self.options.previous) {
            event["previous"] = pack(v)
        }
        let desc = self
            .pool
            .get_message_by_name("payday.WatchEvent")
            .ok_or_else(|| Fault::internal("missing WatchEvent"))?;
        let wire = DynamicMessage::deserialize(desc, event)
            .map_err(|e| Fault::internal(e.to_string()))?
            .encode_to_vec();
        let clean = self.secrets.clean("payday.WatchEvent", &wire)?;
        if clean.len() > 4 << 20 {
            return Err(Fault::exhausted("resource event exceeds 4 MiB"));
        }
        Ok(value::encoded(&clean).as_str().unwrap().to_owned())
    }
}
impl Transaction<'_> {
    /// Evaluate the normal wall on the historical image. Relationships beyond
    /// the image use the same tenant/set predicates and joins as an ordinary Get.
    pub(crate) fn visible_resource(&mut self, e: &Entity, image: &Value) -> Result<Value> {
        let mut q = Query::new("");
        let d = self.db.dialect();
        let mut cols = Vec::new();
        for f in e.fields.iter().filter(|f| f.is_column()) {
            let field = self.field_type(f)?;
            let v = &image[&f.json];
            let v = if f.target.is_empty() {
                v
            } else {
                &v[&field.json]
            };
            let param = if v.is_null() {
                "NULL".into()
            } else {
                q.param(value::bind(field, v)?, d)
            };
            let ty = self.model.scalar(f).map_err(Fault::internal)?.sql(d);
            cols.push(format!("CAST({param} AS {ty}) AS {}", quote(&f.column())));
        }
        let wall = self.wall(e, &mut q)?;
        q.sql = format!(
            "SELECT '1' AS value FROM (SELECT {}) r WHERE {}",
            cols.join(","),
            if wall.is_empty() {
                "TRUE".into()
            } else {
                wall.join(" AND ")
            }
        );
        Ok(json!(!self.db.rows(q)?.is_empty()))
    }
}

pub struct PublicEvents {
    pool: DescriptorPool,
    rules: secrets::Schema,
}
impl PublicEvents {
    pub fn new(descriptors: &[u8], fields: &str) -> Result<Self> {
        Ok(Self {
            pool: DescriptorPool::decode(descriptors).map_err(|e| Fault::invalid(e.to_string()))?,
            rules: secrets::Schema::new(fields)?,
        })
    }
    pub fn pool(&self) -> &DescriptorPool {
        &self.pool
    }
    pub fn public(&self, wire: &[u8]) -> Result<(Value, Vec<u8>)> {
        if wire.len() > 4 << 20 {
            return Err(Fault::exhausted("resource event exceeds 4 MiB"));
        }
        let clean = self.rules.clean("payday.WatchEvent", wire)?;
        let desc = self
            .pool
            .get_message_by_name("payday.WatchEvent")
            .ok_or_else(|| Fault::internal("missing WatchEvent"))?;
        let event = DynamicMessage::decode(desc, clean.as_slice())
            .map_err(|e| Fault::internal(e.to_string()))?;
        Ok((
            serde_json::to_value(event).map_err(|e| Fault::internal(e.to_string()))?,
            clean,
        ))
    }
}
