//! Corrections supersede immutable assertions. Reads choose a knowledge time independently of valid time.
use crate::{Fault, Result, Transaction, value};
use paydirt_model::Entity;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

impl Transaction<'_> {
    pub(crate) fn correct_history(&mut self, req: &Value, whole_event: bool) -> Result<Value> {
        let frame = self
            .frame
            .ok_or_else(|| Fault::precondition("correction requires an operation"))?;
        let tenant = json!({"id":value::encoded(&frame.tenant)});
        let event = self.owned_entity("OWN_EVENT")?.clone();
        let fact = self.owned_entity("OWN_FACT")?.clone();
        let current = self
            .matching(&event, &[("id", &req["event"]), ("tenant", &tenant)], false)?
            .pop()
            .ok_or_else(Fault::missing)?;
        let target = &req["target"];
        if whole_event
            && self
                .matching(&event, &[("id", target), ("tenant", &tenant)], true)?
                .is_empty()
        {
            return Err(Fault::missing());
        }
        let rows = self
            .matching(
                &fact,
                &[
                    (if whole_event { "event_id" } else { "id" }, target),
                    ("tenant", &tenant),
                ],
                true,
            )?
            .into_iter()
            .filter(|r| r["dateSuperseded"].is_null())
            .collect::<Vec<_>>();
        if !whole_event && rows.is_empty() {
            return Err(Fault::missing());
        }
        let correction = req.get("dateStart").filter(|t| !t.is_null());
        let mut slots = BTreeSet::new();
        if let Some(t) = correction {
            super::past::time(t)?;
            for row in &rows {
                if !slots.insert((
                    row["subjectId"].to_string(),
                    row["fieldNumber"].to_string(),
                    row["key"].to_string(),
                )) {
                    return Err(Fault::precondition(
                        "the event said a field from more than one moment",
                    ));
                }
            }
        }
        for row in rows {
            self.supersede(&fact, &row, &current)?;
            if let Some(start) = correction {
                for old in self.matching(
                    &fact,
                    &[
                        ("tenant", &tenant),
                        ("subject_id", &row["subjectId"]),
                        ("field_number", &row["fieldNumber"]),
                        ("key", &row["key"]),
                        ("date_start", start),
                    ],
                    true,
                )? {
                    if old["dateSuperseded"].is_null() {
                        self.supersede(&fact, &old, &current)?;
                    }
                }
                self.insert(&fact,&json!({"tenant":tenant,"subjectId":row["subjectId"],"eventId":current["id"],"domain":row["domain"],"fieldNumber":row["fieldNumber"],"key":row["key"],"value":row["value"],"dateCreated":current["dateCreated"],"dateStart":start}))?;
            }
        }
        Ok(Value::Null)
    }
    pub(crate) fn supersede(&mut self, fact: &Entity, row: &Value, event: &Value) -> Result<()> {
        self.patch(fact,&json!({"ref":{"id":row["id"]},"dateUpdated":row["dateUpdated"],"dateSuperseded":event["dateCreated"],"supersededBy":event["id"]}),"Patch")?;
        Ok(())
    }
    pub(crate) fn statements(&mut self, req: &Value) -> Result<Value> {
        let fact = self.owned_entity("OWN_FACT")?.clone();
        let known = req
            .get("known")
            .filter(|v| !v.is_null())
            .map(super::past::time)
            .transpose()?;
        let mut groups = BTreeMap::new();
        let mut filters = Vec::new();
        for (field, key) in [("subject_id", "subject"), ("event_id", "event")] {
            if let Some(v) = req.get(key).filter(|v| !v.is_null()) {
                filters.push((field, v));
            }
        }
        for row in self.matching(&fact, &filters, false)? {
            let end = row
                .get("dateSuperseded")
                .filter(|v| !v.is_null())
                .map(super::past::time)
                .transpose()?;
            if req["ever"] != true
                && (known
                    .is_some_and(|k| super::past::time(&row["dateCreated"]).is_ok_and(|t| t > k))
                    || end.is_some_and(|e| known.is_none_or(|k| k >= e)))
            {
                continue;
            }
            let group = (
                super::past::time(&row["dateStart"])?,
                row["eventId"].as_str().unwrap().to_owned(),
                row["subjectId"].as_str().unwrap().to_owned(),
            );
            groups.entry(group).or_insert_with(Vec::new).push(row);
        }
        Ok(Value::Array(
            groups
                .into_iter()
                .map(|((start, event, subject), mut rows)| {
                    rows.sort_by(|a, b| {
                        (a["fieldNumber"].as_u64(), a["key"].as_str())
                            .cmp(&(b["fieldNumber"].as_u64(), b["key"].as_str()))
                    });
                    json!({"dateStart":start,"event":event,"subject":subject,"rows":rows})
                })
                .collect(),
        ))
    }
    pub(crate) fn write_facts(&mut self, rows: &[Value]) -> Result<()> {
        let fact = self.owned_entity("OWN_FACT")?.clone();
        let slot = |r: &Value| -> Result<_> {
            Ok((
                r["subjectId"]
                    .as_str()
                    .ok_or_else(|| Fault::invalid("missing fact subject"))?
                    .to_owned(),
                r["fieldNumber"].as_u64(),
                r["key"].as_str().unwrap_or_default().to_owned(),
                super::past::time(&r["dateStart"])?,
            ))
        };
        let subject = fact.field("subject_id").map_err(Fault::internal)?;
        let valid = fact.field("date_start").map_err(Fault::internal)?;
        let superseded = paydirt_model::quote(
            &fact
                .field("date_superseded")
                .map_err(Fault::internal)?
                .column(),
        );
        let mut rest = rows;
        while !rest.is_empty() {
            // A second statement of the same slot must see its predecessor as a standing row.
            let mut page = BTreeMap::new();
            for row in rest.iter().take(128) {
                let key = slot(row)?;
                if page.contains_key(&key) {
                    break;
                }
                page.insert(key, row);
            }
            let (writes, later) = rest.split_at(page.len());
            rest = later;
            let mut query = crate::driver::Query::new("");
            let mut clauses = self.wall(&fact, &mut query)?;
            for (f, key) in [(subject, "subjectId"), (valid, "dateStart")] {
                let values = writes
                    .iter()
                    .filter_map(|r| r[key].as_str())
                    .collect::<BTreeSet<_>>();
                let params = values
                    .into_iter()
                    .map(|s| value::bind(f, &json!(s)).map(|v| query.param(v, self.db.dialect())))
                    .collect::<Result<Vec<_>>>()?;
                clauses.push(format!(
                    "r.{} IN ({})",
                    paydirt_model::quote(&f.column()),
                    params.join(",")
                ));
            }
            clauses.push(format!("r.{superseded} IS NULL"));
            let lock = if self.db.dialect() == paydirt_model::Dialect::Postgres {
                " FOR UPDATE"
            } else {
                ""
            };
            query.sql = format!(
                "SELECT {} AS value FROM {} r WHERE {} ORDER BY r.{}{lock}",
                self.project_sql(&fact)?,
                paydirt_model::quote(&fact.table()),
                clauses.join(" AND "),
                paydirt_model::quote(&fact.key().map_err(Fault::internal)?.column())
            );
            for raw in self.db.rows(query)? {
                let old = self.decode_row(&fact, &raw)?;
                if let Some(new) = page.get(&slot(&old)?) {
                    self.supersede(
                        &fact,
                        &old,
                        &json!({"id":new["eventId"],"dateCreated":new["dateCreated"]}),
                    )?;
                }
            }
            self.insert_items(&fact, writes, writes.len() > 1)?;
        }
        Ok(())
    }
    pub(crate) fn event_children(&mut self, e: &Entity, req: &Value) -> Result<Value> {
        let children = req["children"]
            .as_array()
            .ok_or_else(|| Fault::invalid("expected children"))?;
        let event = self.owned_entity("OWN_EVENT")?.clone();
        let mut out = Vec::with_capacity(children.len());
        for (page_no, page) in children.chunks(128).enumerate() {
            let mut pending = Vec::new();
            let mut prepared = Vec::new();
            for (index, child) in page.iter().enumerate() {
                let mut op = child.clone();
                op["cause"] = req["event"].clone();
                let entity = if let Some(name) = child["entity"].as_str() {
                    self.model.entity(name).map_err(Fault::invalid)?
                } else {
                    e
                };
                let row = self.prepare_event(entity, &op).map_err(|mut err| {
                    err.message = format!("child {}: {}", page_no * 128 + index, err.message);
                    err
                })?;
                if row["done"] != true {
                    pending.push(row.clone());
                }
                prepared.push(row);
            }
            let mut inserted = self.insert_items(&event, &pending, true)?.into_iter();
            let mut facts = Vec::new();
            for (offset, (child, row)) in page.iter().zip(prepared).enumerate() {
                if row["done"] == true {
                    out.push(row);
                    continue;
                }
                let row = inserted
                    .next()
                    .ok_or_else(|| Fault::internal("incomplete child insert"))?;
                if !child["after"].is_null() {
                    let entity = if let Some(name) = child["entity"].as_str() {
                        self.model.entity(name).map_err(Fault::invalid)?
                    } else {
                        e
                    };
                    facts.extend(self.prepare_facts(entity, &row, child).map_err(|mut err| {
                        err.message = format!("child {}: {}", page_no * 128 + offset, err.message);
                        err
                    })?);
                }
                out.push(row);
            }
            self.write_facts(&facts)?;
        }
        Ok(Value::Array(out))
    }
}
