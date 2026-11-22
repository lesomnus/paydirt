//! Application events and the valid-time and recorded-time history of their subjects.
use crate::{Fault, Result, Transaction, driver::Query, value};
use chrono::{DateTime, Utc};
use paydirt_model::{Entity, Field, quote};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

type Slot = (u32, String);
fn time(v: &Value) -> Result<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Fault::invalid("expected RFC3339 timestamp"))
}
fn canonical(f: &Field, v: &Value) -> Result<String> {
    if v.is_null() {
        return Ok("null".into());
    }
    if !f.target.is_empty() || f.kind == "UUID" {
        let id = value::bytes(v.get("id").unwrap_or(v))?;
        return Ok(json!(
            paydirt::format_id(&id).ok_or_else(|| Fault::invalid("invalid fact UUID"))?
        )
        .to_string());
    }
    if matches!(
        f.kind.as_str(),
        "INT64" | "SINT64" | "SFIXED64" | "UINT64" | "FIXED64"
    ) && let Some(n) = v.as_str()
    {
        return Ok(n.to_owned());
    }
    Ok(v.to_string())
}
fn changes(
    entity: &Entity,
    policy: &crate::Policy,
    before: &Value,
    after: &Value,
) -> Result<BTreeMap<Slot, String>> {
    let mut out = BTreeMap::new();
    for f in &entity.fields {
        if f.name == entity.key || f.version || f.immutable {
            continue;
        }
        let (a, b) = (&before[&f.json], &after[&f.json]);
        if a == b {
            continue;
        }
        let private = policy
            .fields
            .get(&f.name)
            .is_some_and(|v| v["secret"] == true || v["personal"] == true);
        if f.collection == "map" && !private {
            let keys: BTreeSet<_> = [a, b]
                .into_iter()
                .filter_map(Value::as_object)
                .flat_map(|m| m.keys())
                .collect();
            for key in keys {
                if a[key] != b[key] {
                    out.insert((f.number, key.clone()), b[key].to_string());
                }
            }
        } else {
            let clear = b.is_null()
                || (f.collection == "map" && b.as_object().is_some_and(|m| m.is_empty()))
                || (f.collection == "list" && b.as_array().is_some_and(|a| a.is_empty()));
            let v = if clear {
                "null".into()
            } else if private {
                String::new()
            } else {
                canonical(f, b)?
            };
            out.insert((f.number, String::new()), v);
        }
    }
    Ok(out)
}
fn fold(
    rows: &[Value],
    at: DateTime<Utc>,
    known: Option<DateTime<Utc>>,
) -> Result<BTreeMap<Slot, Value>> {
    let mut values = BTreeMap::<Slot, (DateTime<Utc>, DateTime<Utc>, Vec<u8>, Value)>::new();
    for row in rows {
        let valid = time(&row["dateStart"])?;
        let recorded = time(&row["dateCreated"])?;
        let end = row
            .get("dateSuperseded")
            .filter(|v| !v.is_null())
            .map(time)
            .transpose()?;
        if valid > at
            || known.is_some_and(|k| recorded > k)
            || end.is_some_and(|t| known.is_none_or(|k| k >= t))
        {
            continue;
        }
        let slot = (
            row["fieldNumber"]
                .as_u64()
                .ok_or_else(|| Fault::internal("invalid fact field"))? as u32,
            row["key"].as_str().unwrap_or("").into(),
        );
        let id = value::bytes(&row["id"])?;
        let replace = values
            .get(&slot)
            .is_none_or(|(v, r, i, _)| (&valid, &recorded, &id) > (v, r, i));
        if replace {
            values.insert(slot, (valid, recorded, id, row.clone()));
        }
    }
    Ok(values
        .into_iter()
        .filter_map(|(s, (_, _, _, r))| (r["value"] != "null").then_some((s, r)))
        .collect())
}
impl Transaction<'_> {
    pub(crate) fn owned_entity(&self, own: &str) -> Result<&Entity> {
        let (name, _) = self
            .policies
            .iter()
            .find(|(_, p)| p.entity["own"] == own)
            .ok_or_else(|| Fault::unsupported(format!("application has no {own} schema")))?;
        self.model.entity(name).map_err(Fault::internal)
    }
    pub(crate) fn matching(
        &mut self,
        e: &Entity,
        fields: &[(&str, &Value)],
        lock: bool,
    ) -> Result<Vec<Value>> {
        let mut q = Query::new("");
        let mut parts = self.wall(e, &mut q)?;
        for (name, v) in fields {
            let field = e.field(name).map_err(Fault::internal)?;
            let value = self.resolved(field, v)?;
            let arg = q.param(value, self.db.dialect());
            parts.push(format!("r.{}={arg}", quote(&field.column())));
        }
        let order = quote(&e.key().map_err(Fault::internal)?.column());
        let suffix = if lock && self.db.dialect() == paydirt_model::Dialect::Postgres {
            " FOR UPDATE"
        } else {
            ""
        };
        q.sql = format!(
            "SELECT {} AS value FROM {} r WHERE {} ORDER BY r.{order}{suffix}",
            self.project_sql(e)?,
            quote(&e.table()),
            if parts.is_empty() {
                "TRUE".into()
            } else {
                parts.join(" AND ")
            }
        );
        self.db
            .rows(q)?
            .iter()
            .map(|r| self.decode_row(e, r))
            .collect()
    }
    pub(crate) fn begin_event(&mut self, e: &Entity, req: &Value) -> Result<Value> {
        let frame = self
            .frame
            .ok_or_else(|| Fault::precondition("events require an application transaction"))?;
        if frame.tenant.is_empty() {
            return Err(Fault::precondition("events require a tenant"));
        }
        let tenant = value::encoded(&frame.tenant);
        let event = self.owned_entity("OWN_EVENT")?.clone();
        let defs = &self
            .policies
            .values()
            .find(|p| p.entity["own"] == "OWN_HOLDER")
            .ok_or_else(|| Fault::internal("missing application schema"))?
            .application["eventTypes"];
        let full = req["value"]["@type"]
            .as_str()
            .and_then(|s| s.rsplit('/').next())
            .ok_or_else(|| Fault::invalid("event payload required"))?;
        let kinds = defs
            .as_object()
            .into_iter()
            .flat_map(|v| v.iter())
            .filter(|(_, n)| n.as_str() == Some(full))
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>();
        let kind = match req["kind"].as_str().filter(|k| !k.is_empty()) {
            Some(k) if kinds.contains(&k) => k,
            None if kinds.len() == 1 => kinds[0],
            _ => {
                return Err(Fault::invalid(
                    "expected a declared event kind for this payload",
                ));
            }
        };
        let mut data = json!({"tenant":{"id":tenant},"kind":kind,"traceId":value::encoded(&frame.trace),"value":req["value"],"reason":req["reason"].as_str().unwrap_or_default()});
        if let Some(actor) = &frame.actor {
            data["actorId"] = value::encoded(actor)
        }
        if let Some(key) = req.get("id").filter(|v| !v.is_null()) {
            let bytes = value::bytes(key)?;
            let domain = self.policies[&event.name].entity["domain"]
                .as_u64()
                .unwrap() as u8;
            crate::domain::Domains::from_policies(self.policies)?.check_id(&bytes, domain)?;
            self.past_lock(&bytes)?;
            if !self
                .matching(
                    &event,
                    &[("id", key), ("tenant", &json!({"id":tenant}))],
                    false,
                )?
                .is_empty()
            {
                return Ok(json!({"done":true,"id":key}));
            }
            data["id"] = key.clone();
        }
        if let Some(subject) = req.get("subject").filter(|v| !v.is_null()) {
            let bytes = value::bytes(subject)?;
            crate::domain::Domains::from_policies(self.policies)?.check_id(
                &bytes,
                self.policies[&e.name].entity["domain"]
                    .as_u64()
                    .ok_or_else(|| Fault::invalid("subject entity has no domain"))?
                    as u8,
            )?;
            match self.lookup(
                e,
                &json!({&e.key().map_err(Fault::internal)?.json:subject}),
                true,
            ) {
                Ok(row) => {
                    if self.object_tenant(e, &row)? != tenant {
                        return Err(Fault::missing());
                    }
                }
                Err(err) if err.code == "not_found" => {
                    let mut q = Query::new("");
                    let k = q.param(crate::driver::Bind::Bytes(bytes), self.db.dialect());
                    q.sql = format!(
                        "SELECT '1' AS value FROM {} WHERE {}={k}",
                        quote(&e.table()),
                        quote(&e.key().map_err(Fault::internal)?.column())
                    );
                    if !self.db.rows(q)?.is_empty() {
                        return Err(Fault::missing());
                    }
                }
                Err(err) => return Err(err),
            }
            data["subjectId"] = subject.clone();
        }
        if let Some(cause) = req.get("cause").filter(|v| !v.is_null()) {
            data["causeId"] = cause.clone();
        }
        let start = req
            .get("dateStart")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| self.inputs.stamp());
        time(&start)?;
        data["dateOccurred"] = start;
        self.insert(&event, &data)
    }
    pub(crate) fn past_lock(&mut self, id: &[u8]) -> Result<()> {
        if self.db.dialect() == paydirt_model::Dialect::Postgres {
            let key: i64 = i64::from_be_bytes(
                id.get(..8)
                    .ok_or_else(|| Fault::invalid("invalid operation UUID"))?
                    .try_into()
                    .unwrap(),
            ) ^ i64::from_be_bytes(
                id.get(8..16)
                    .ok_or_else(|| Fault::invalid("invalid operation UUID"))?
                    .try_into()
                    .unwrap(),
            );
            let mut q = Query::new("");
            let p = q.param(crate::driver::Bind::Big(key), self.db.dialect());
            q.sql = format!("SELECT '1' AS value FROM pg_advisory_xact_lock({p})");
            self.db.rows(q)?;
        }
        Ok(())
    }
    pub(crate) fn event_facts(
        &mut self,
        e: &Entity,
        event_id: &Value,
        req: &Value,
    ) -> Result<Value> {
        let event_schema = self.owned_entity("OWN_EVENT")?.clone();
        let event = self.lookup(&event_schema, &json!({"id":event_id}), false)?;
        let (before, after) = (&req["before"], &req["after"]);
        let subject = &after["id"];
        if !before.is_null() && before["id"] != *subject {
            return Err(Fault::invalid("event contains different subjects"));
        }
        let row = self.lookup(e, &json!({"id":subject}), true)?;
        let tenant = self.object_tenant(e, &row)?;
        if tenant != event["tenant"]["id"] {
            return Err(Fault::missing());
        }
        let start = req
            .get("dateStart")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| event["dateOccurred"].clone());
        time(&start)?;
        let changes = changes(e, &self.policies[&e.name], before, after)?;
        let fact = self.owned_entity("OWN_FACT")?.clone();
        for ((n, key), val) in changes {
            let existing = self.matching(
                &fact,
                &[
                    ("subject_id", subject),
                    ("field_number", &json!(n)),
                    ("key", &json!(key)),
                    ("date_start", &start),
                ],
                true,
            )?;
            for old in existing
                .into_iter()
                .filter(|r| r.get("dateSuperseded").is_none())
            {
                self.patch(&fact,&json!({"ref":{"id":old["id"]},"dateUpdated":old["dateUpdated"],"dateSuperseded":event["dateCreated"],"supersededBy":event["id"]}),"Patch")?;
            }
            self.insert(&fact,&json!({"tenant":{"id":tenant},"subjectId":subject,"eventId":event["id"],"fieldNumber":n,"domain":self.policies[&e.name].entity["domain"],"key":key,"value":val,"dateStart":start}))?;
        }
        Ok(event)
    }
    pub(crate) fn say(&mut self, e: &Entity, req: &Value) -> Result<Value> {
        let mut op = req.clone();
        op["subject"] = req["after"]["id"].clone();
        let event = self.begin_event(e, &op)?;
        if event["done"] == true {
            return Ok(event);
        }
        self.event_facts(e, &event["id"], req)
    }

    pub(crate) fn past_state(&mut self, e: &Entity, req: &Value) -> Result<Value> {
        let subject = self.lookup(e, &req["ref"], false)?;
        let fact = self.owned_entity("OWN_FACT")?.clone();
        let rows = self.matching(&fact, &[("subject_id", &subject["id"])], false)?;
        let at = time(&req["at"])?;
        let known = req
            .get("known")
            .filter(|v| !v.is_null())
            .map(time)
            .transpose()?;
        Ok(
            json!({"subject":subject["id"],"facts":fold(&rows,at,known)?.into_values().collect::<Vec<_>>()}),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn personal_map_never_keeps_its_keys_and_identity_never_becomes_a_fact() {
        let entity: Entity = serde_json::from_value(
            json!({"name":"app.Person","key":"id","indexes":[],"fields":[
                {"name":"id","json":"id","number":1,"kind":"UUID","immutable":true},
                {"name":"name","json":"name","number":5,"kind":"STRING"},
                {"name":"contacts","json":"contacts","number":25,"kind":"JSON","collection":"map"},
                {"name":"password","json":"password","number":30,"kind":"STRING"}
            ]}),
        )
        .unwrap();
        let policy: crate::Policy = serde_json::from_value(
            json!({"fields":{"contacts":{"personal":true},"password":{"secret":true}}}),
        )
        .unwrap();
        let record =
            json!({"id":"never","name":"public","contacts":{"person":"phone"},"password":"secret"});
        let slots = changes(&entity, &policy, &Value::Null, &record).unwrap();
        assert_eq!(
            slots.into_values().collect::<Vec<_>>(),
            [r#""public""#, "", ""]
        );
        let clear = changes(&entity, &policy, &record, &Value::Null).unwrap();
        assert_eq!(clear[&(25, "".into())], "null");
    }
    #[test]
    fn historical_state_uses_both_valid_time_and_knowledge_time() {
        let day = |n| format!("2026-10-{n:02}T00:00:00Z");
        let row = |value: &str, valid, recorded, until: Option<u8>| json!({"id":value::encoded(&paydirt::mint(18)),"value":value,"fieldNumber":3,"key":"","dateStart":day(valid),"dateCreated":day(recorded),"dateSuperseded":until.map(day)});
        let rows = vec![
            row("A", 1, 2, None),
            row("B", 5, 6, Some(9)),
            row("B", 3, 9, None),
            row("null", 15, 16, None),
        ];
        let state = |at, known: Option<u8>| {
            fold(
                &rows,
                day(at).parse().unwrap(),
                known.map(|d| day(d).parse().unwrap()),
            )
            .unwrap()
            .into_values()
            .next()
            .map(|v| v["value"].clone())
        };
        assert_eq!(state(4, None), Some(json!("B")));
        assert_eq!(state(4, Some(7)), Some(json!("A")));
        assert_eq!(state(5, Some(7)), Some(json!("B")));
        assert_eq!(state(16, None), None);
    }
}

impl Transaction<'_> {
    pub(crate) fn redact_reason(&mut self, req: &Value) -> Result<Value> {
        let event = self.owned_entity("OWN_EVENT")?.clone();
        let row = self.lookup(&event, &req["ref"], true)?;
        if row["reason"].as_str().unwrap_or_default().is_empty() {
            return Ok(Value::Null);
        }
        self.patch(
            &event,
            &json!({"ref":req["ref"],"reason":"","dateUpdated":row["dateUpdated"]}),
            "Redact",
        )?;
        Ok(Value::Null)
    }
}
