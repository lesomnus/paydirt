//! Access subjects are discovered from declared message types. Application-owned Any values are opaque.
use crate::{Fault, Frame, Result, Scope, Transaction, audit_content::Recorder, value};
use buffa_reflect::Kind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub frame: Frame,
    pub request: Value,
    pub input: String,
    pub output: String,
    pub answer: Option<Value>,
    pub result: String,
    pub reason: Option<String>,
    pub written: BTreeSet<String>,
}
type Subjects = BTreeMap<String, BTreeSet<String>>;
fn id(v: &Value) -> Option<String> {
    let bytes = value::bytes(v).ok()?;
    (bytes.len() == 16 && bytes.iter().any(|b| *b != 0))
        .then(|| value::encoded(&bytes).as_str().unwrap().to_owned())
}
impl Recorder {
    fn subjects(
        &self,
        policies: &crate::Policies,
        ty: &str,
        doc: &Value,
        refs: bool,
        tenant: &str,
        groups: &mut Subjects,
        unpack: bool,
        depth: usize,
    ) -> Result<()> {
        if depth > 100 {
            return Err(Fault::exhausted("access message too deeply nested"));
        }
        if ty == "google.protobuf.Any" {
            return Ok(());
        }
        let entity = if refs {
            ty.strip_suffix("Ref")
        } else {
            Some(ty)
        };
        if let Some(p) = entity.and_then(|n| policies.get(n))
            && p.fields.values().any(|f| f["personal"] == true)
            && let Some(subject) = id(&doc["id"])
        {
            let owner = if refs {
                None
            } else {
                id(&doc["tenant"]["id"]).or_else(|| id(&doc["tenantId"]))
            };
            groups
                .entry(owner.unwrap_or_else(|| tenant.to_owned()))
                .or_default()
                .insert(subject);
        }
        let Some(d) = self.pool.get_message_by_name(ty) else {
            return Ok(());
        };
        for f in d.fields() {
            let Some(v) = doc.get(f.json_name()).filter(|v| !v.is_null()) else {
                continue;
            };
            let Kind::Message(m) = f.kind() else { continue };
            let mut each = |v: &Value, ty2: &str| -> Result<()> {
                if unpack
                    && ty == "payday.WatchEvent"
                    && matches!(f.name(), "value" | "previous")
                    && let Some(name) = v["@type"].as_str().and_then(|v| v.rsplit('/').next())
                {
                    self.subjects(policies, name, v, refs, tenant, groups, false, depth + 1)
                } else {
                    self.subjects(policies, ty2, v, refs, tenant, groups, unpack, depth + 1)
                }
            };
            if f.is_map() {
                if let Some(Kind::Message(target)) = m.get_field_by_number(2).map(|f| f.kind()) {
                    for item in v.as_object().into_iter().flat_map(|m| m.values()) {
                        each(item, target.full_name())?;
                    }
                }
            } else if f.is_list() {
                for item in v.as_array().into_iter().flatten() {
                    each(item, m.full_name())?;
                }
            } else {
                each(v, m.full_name())?;
            }
        }
        Ok(())
    }
}
impl Transaction<'_> {
    fn access_rows(
        &mut self,
        frame: &Frame,
        kind: &str,
        result: &str,
        groups: Subjects,
        detail: Value,
    ) -> Result<()> {
        let Some((name, policy)) = self
            .policies
            .iter()
            .find(|(_, p)| p.entity["own"] == "OWN_ACCESS")
        else {
            return Err(Fault::internal("missing Access schema"));
        };
        let e = self.model.entity(name).map_err(Fault::internal)?;
        let actor = value::encoded(frame.actor.as_deref().unwrap_or(&[0; 16]));
        let tenant = if frame.tenant.is_empty() {
            value::encoded(&[0; 16])
        } else {
            value::encoded(&frame.tenant)
        };
        for (owner, subjects) in groups {
            if subjects.is_empty() {
                continue;
            }
            let n = subjects.len();
            let ids: Vec<_> = subjects.into_iter().take(1000).collect();
            let mut row = json!({"actorId":actor,"actorTenantId":tenant,"tenantId":owner,"traceId":value::encoded(&frame.trace),"action":frame.action,"kind":kind,"result":result,"subjects":ids,"subjectCount":n,"domain":policy.entity["domain"]});
            if !detail.is_null() {
                row["value"] = detail.clone()
            }
            self.insert(e, &row)?;
        }
        Ok(())
    }
    pub(crate) fn observe(&mut self, o: &Observation) -> Result<Value> {
        let Some(rec) = self.trail.filter(|r| r.options.access) else {
            return Ok(Value::Null);
        };
        let tenant = value::encoded(if o.frame.tenant.is_empty() {
            &[0; 16]
        } else {
            &o.frame.tenant
        });
        let mut groups = Subjects::new();
        let (ty, doc, refs) = match &o.answer {
            Some(v) => (&o.output, v, false),
            None => (&o.input, &o.request, true),
        };
        rec.subjects(
            self.policies,
            ty,
            doc,
            refs,
            tenant.as_str().unwrap(),
            &mut groups,
            true,
            0,
        )?;
        for ids in groups.values_mut() {
            ids.retain(|s| !o.written.contains(s))
        }
        if groups.values().all(BTreeSet::is_empty) {
            return Ok(Value::Null);
        }
        let mut detail = json!({});
        if o.result == "ok" && o.written.is_empty() && !o.request.is_null() {
            let req = rec.message(&o.input, o.request.clone())?.encode_to_vec();
            if !req.is_empty() && req.len() <= 8192 {
                detail["request"] = value::encoded(&req)
            }
        }
        if let Some(reason) = &o.reason {
            detail["reason"] = json!(reason);
        }
        let bytes = rec.message("payday.AccessDetail", detail)?.encode_to_vec();
        self.access_rows(
            &o.frame,
            if o.reason.is_some() {
                "download"
            } else {
                "read"
            },
            &o.result,
            groups,
            value::encoded(&bytes),
        )?;
        Ok(Value::Null)
    }
    pub(crate) fn access_write(
        &mut self,
        frame: &Frame,
        tenant: &Value,
        object: &Value,
    ) -> Result<()> {
        if let (Some(t), Some(o)) = (id(tenant), id(object)) {
            self.access_rows(frame, "write", "ok", [(t, [o].into())].into(), Value::Null)?;
        }
        Ok(())
    }
}
impl crate::Store {
    pub(crate) fn observe(&mut self, o: Observation) -> Result<Value> {
        self.transaction(&Scope::Internal, |tx| tx.observe(&o))
    }
}
