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
pub(crate) type Subjects = BTreeMap<String, BTreeSet<String>>;
fn id(v: &Value) -> Option<String> {
    let bytes = value::bytes(v).ok()?;
    (bytes.len() == 16 && bytes.iter().any(|b| *b != 0))
        .then(|| value::encoded(&bytes).as_str().unwrap().to_owned())
}
impl Recorder {
    fn carries(
        &self,
        policies: &crate::Policies,
        ty: &str,
        doc: &Value,
        refs: bool,
    ) -> Result<bool> {
        let Some(policy) = policies.get(ty) else {
            return Ok(false);
        };
        let Some(desc) = self.pool.get_message_by_name(ty) else {
            return Ok(false);
        };
        let mut fields = serde_json::Map::new();
        for (name, _) in policy.fields.iter().filter(|(_, p)| p["personal"] == true) {
            if refs {
                return Ok(true);
            }
            let Some(field) = desc.get_field_by_name(name) else {
                continue;
            };
            if let Some(v) = doc.get(field.json_name()).filter(|v| !v.is_null()) {
                fields.insert(field.json_name().to_owned(), v.clone());
            }
        }
        if fields.is_empty() {
            return Ok(false);
        }
        let msg = self.message(ty, Value::Object(fields))?;
        Ok(msg.fields().next().is_some())
    }
    pub(crate) fn subjects(
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
        if let Some(entity) = entity
            && self.carries(policies, entity, doc, refs)?
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
fn access_rows(
    domain: &Value,
    frame: &Frame,
    kind: &str,
    result: &str,
    groups: Subjects,
    detail: Value,
) -> Vec<Value> {
    let mut rows = Vec::new();
    let actor = value::encoded(
        if frame.tenant.is_empty() {
            frame.account.as_deref()
        } else {
            frame.actor.as_deref()
        }
        .unwrap_or(&[0; 16]),
    );
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
        let mut row = json!({"actorId":actor,"actorTenantId":tenant,"tenantId":owner,"traceId":value::encoded(&frame.trace),"action":frame.action,"kind":kind,"result":result,"subjects":ids,"subjectCount":n,"domain":domain});
        if !detail.is_null() {
            row["value"] = detail.clone()
        }
        rows.push(row);
    }
    rows
}
impl Recorder {
    fn observation(
        &self,
        policies: &crate::Policies,
        domain: &Value,
        o: &Observation,
    ) -> Result<Vec<Value>> {
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
        self.subjects(
            policies,
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
            return Ok(vec![]);
        }
        let mut detail = json!({});
        if o.result == "ok" && o.written.is_empty() && !o.request.is_null() {
            let req = self.message(&o.input, o.request.clone())?.encode_to_vec();
            if !req.is_empty() && req.len() <= 8192 {
                detail["request"] = value::encoded(&req)
            }
        }
        if let Some(address) = &o.frame.address {
            detail["address"] = json!(address);
        }
        if let Some(reason) = &o.reason {
            detail["reason"] = json!(reason);
        }
        let bytes = self.message("payday.AccessDetail", detail)?.encode_to_vec();
        Ok(access_rows(
            domain,
            &o.frame,
            if o.reason.is_some() {
                "download"
            } else {
                "read"
            },
            &o.result,
            groups,
            value::encoded(&bytes),
        ))
    }
}

impl Transaction<'_> {
    fn write_access(&mut self, rows: &[Value]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let e = self.owned_entity("OWN_ACCESS")?.clone();
        for row in rows {
            self.insert(&e, row)?;
        }
        Ok(())
    }
    pub(crate) fn flush_written(&mut self, frame: &Frame, groups: Subjects) -> Result<()> {
        if groups.is_empty() {
            return Ok(());
        }
        let detail = if let Some(address) = &frame.address {
            let recorder = self
                .trail
                .ok_or_else(|| Fault::internal("missing access recorder"))?;
            value::encoded(
                &recorder
                    .message("payday.AccessDetail", json!({"address":address}))?
                    .encode_to_vec(),
            )
        } else {
            Value::Null
        };
        let domain = &self
            .policies
            .iter()
            .find(|(_, p)| p.entity["own"] == "OWN_ACCESS")
            .ok_or_else(|| Fault::internal("missing Access schema"))?
            .1
            .entity["domain"];
        self.write_access(&access_rows(domain, frame, "write", "ok", groups, detail))
    }
}
impl crate::Store {
    /// Executes a host command under a named operator. Personal reads are
    /// recorded synchronously, including their local origin.
    pub fn operator_as(
        &mut self,
        call: crate::Call,
        actor: [u8; 16],
        tenant: [u8; 16],
        address: String,
    ) -> Result<Value> {
        if actor == [0; 16] || !paydirt::has_domain(&tenant, 1) || address.trim().is_empty() {
            return Err(Fault::invalid("expected an operator, tenant and origin"));
        }
        let mut frame = crate::entitle::deployment(
            tenant.to_vec(),
            &format!("/{}Service/{}", call.entity, call.method),
        );
        frame.actor = Some(actor.to_vec());
        frame.address = Some(address);
        let method = self
            .policies
            .get(&call.entity)
            .and_then(|p| p.methods.get(&call.method))
            .cloned()
            .ok_or_else(|| Fault::invalid("unknown operator method"))?;
        let request = call.request.clone();
        let (answer, written) = self.transaction_as(&Scope::Internal, Some(&frame), |tx| {
            let answer = tx.call(call)?;
            let written = tx
                .pending_access
                .as_deref()
                .into_iter()
                .flat_map(|m| m.values().flatten().cloned())
                .collect();
            Ok((answer, written))
        })?;
        self.observe(
            Observation {
                frame,
                request,
                input: method["input"]
                    .as_str()
                    .unwrap_or_default()
                    .trim_start_matches('.')
                    .into(),
                output: method["output"]
                    .as_str()
                    .unwrap_or_default()
                    .trim_start_matches('.')
                    .into(),
                answer: Some(answer.clone()),
                result: "ok".into(),
                reason: None,
                written,
            },
            false,
        )
        .map_err(|_| Fault::unavailable("operator access could not be recorded"))?;
        Ok(answer)
    }
    pub(crate) fn observe(&mut self, o: Observation, buffer: bool) -> Result<Value> {
        self.observe_with_writes(o, buffer, Default::default())
    }
    pub(crate) fn observe_with_writes(
        &mut self,
        mut o: Observation,
        buffer: bool,
        subjects: Subjects,
    ) -> Result<Value> {
        let Some(rec) = self.trail.as_ref().filter(|r| r.options.access) else {
            return Ok(Value::Null);
        };
        let domain = &self
            .policies
            .iter()
            .find(|(_, p)| p.entity["own"] == "OWN_ACCESS")
            .ok_or_else(|| Fault::internal("missing Access schema"))?
            .1
            .entity["domain"];
        o.written.extend(subjects.values().flatten().cloned());
        let mut rows = rec.observation(&self.policies, domain, &o)?;
        if !subjects.is_empty() {
            if o.result == "ok" {
                return Err(Fault::invalid("expected a failed call"));
            }
            let detail = if let Some(address) = &o.frame.address {
                value::encoded(
                    &rec.message("payday.AccessDetail", json!({"address":address}))?
                        .encode_to_vec(),
                )
            } else {
                Value::Null
            };
            rows.extend(access_rows(
                domain, &o.frame, "write", &o.result, subjects, detail,
            ));
        }
        for row in &mut rows {
            row["dateCreated"] = self.inputs.stamp();
        }
        if buffer && o.reason.is_none() {
            if self.pending_reads.len() + rows.len() > 2048 {
                return Err(Fault::exhausted("access record buffer is full"));
            }
            self.pending_reads.extend(rows);
            return Ok(json!(self.pending_reads.len()));
        }
        if !rows.is_empty() {
            let committed = self.committed.take();
            let result = self.transaction(&Scope::Internal, |tx| tx.write_access(&rows));
            self.committed = committed;
            result?;
        }
        Ok(Value::Null)
    }
    pub fn flush_access(&mut self) -> Result<usize> {
        if self.pending_reads.is_empty() {
            return Ok(0);
        }
        let rows = std::mem::take(&mut self.pending_reads);
        let committed = self.committed.take();
        let result = self.transaction(&Scope::Internal, |tx| tx.write_access(&rows));
        self.committed = committed;
        if let Err(err) = result {
            self.pending_reads = rows;
            return Err(err);
        }
        Ok(rows.len())
    }
}
