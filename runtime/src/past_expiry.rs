//! Tenant-local expiration preserves the baseline required for bitemporal reads.
use crate::{Call, Fault, Frame, Packet, Result, Store, Transaction, driver::Query, value};
use chrono::{DateTime, TimeDelta, Utc};
use paydirt_model::quote;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Window {
    pub keep_ms: u64,
    pub holds: Vec<String>,
    pub keep_kinds: BTreeSet<String>,
    pub keep_facts: Vec<KeepFact>,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct KeepFact {
    pub domain: u32,
    pub field: u32,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub values: Vec<String>,
}
#[derive(Serialize, Deserialize, Default)]
pub struct Expired {
    pub facts: Vec<String>,
    pub events: Vec<String>,
    pub date_before: Option<DateTime<Utc>>,
}
fn time(v: &Value) -> Result<DateTime<Utc>> {
    v.as_str()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| Fault::internal("invalid history timestamp"))
}
fn text(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}
impl Store {
    pub fn expire_past(&mut self, tenant: [u8; 16], window: &Window, dry: bool) -> Result<Expired> {
        let frame: Frame =
            serde_json::from_value(json!({"tenant":tenant,"action":"payday.past.expire"}))
                .map_err(|e| Fault::internal(e.to_string()))?;
        let event = self
            .policies
            .iter()
            .find(|(_, v)| v.entity["own"] == "OWN_EVENT")
            .map(|(n, _)| n.clone())
            .ok_or_else(|| Fault::precondition("application has no past"))?;
        let session = hex::encode(paydirt::mint(0));
        self.handle(Packet::Begin {
            id: session.clone(),
            frame,
        })?;
        let result = self.handle(Packet::Call {
            id: session.clone(),
            call: Call {
                entity: event,
                method: "$expire_past".into(),
                request: json!({"window":window,"dry":dry}),
            },
        });
        self.handle(Packet::Finish {
            id: session,
            commit: result.is_ok() && !dry,
        })?;
        serde_json::from_value(result?).map_err(|e| Fault::internal(e.to_string()))
    }
}
impl Transaction<'_> {
    pub(crate) fn expire_history(&mut self, req: &Value) -> Result<Value> {
        let frame = self
            .frame
            .ok_or_else(|| Fault::precondition("expiry requires a transaction"))?;
        if frame.action != "payday.past.expire" {
            return Err(Fault::denied("host expiry required"));
        }
        let window: Window = serde_json::from_value(req["window"].clone())
            .map_err(|e| Fault::invalid(e.to_string()))?;
        let mut out = Expired::default();
        if window.keep_ms == 0 || !window.holds.is_empty() {
            return Ok(json!(out));
        }
        for rule in &window.keep_facts {
            for v in &rule.values {
                let parsed: Value = serde_json::from_str(v)
                    .map_err(|_| Fault::invalid("invalid retained fact value"))?;
                if parsed.to_string() != *v {
                    return Err(Fault::invalid("retained fact value must be canonical JSON"));
                }
            }
        }
        self.past_lock(&frame.tenant)?;
        let delta = TimeDelta::try_milliseconds(
            window
                .keep_ms
                .try_into()
                .map_err(|_| Fault::invalid("invalid history window"))?,
        )
        .ok_or_else(|| Fault::invalid("invalid history window"))?;
        let cut = (self.inputs.clock)()
            .checked_sub_signed(delta)
            .ok_or_else(|| Fault::invalid("invalid history cutoff"))?;
        out.date_before = Some(cut);
        let fact = self.owned_entity("OWN_FACT")?.clone();
        let event = self.owned_entity("OWN_EVENT")?.clone();
        let tenant = json!({"id":value::encoded(&frame.tenant)});
        let facts = self.matching(&fact, &[("tenant", &tenant)], true)?;
        let events = self.matching(&event, &[("tenant", &tenant)], true)?;
        let slot = |r: &Value| {
            (
                text(&r["subjectId"]).to_owned(),
                r["fieldNumber"].as_u64().unwrap_or_default(),
                text(&r["key"]).to_owned(),
            )
        };
        let mut latest = BTreeMap::new();
        for f in &facts {
            let start = time(&f["dateStart"])?;
            if start <= cut && time(&f["dateCreated"])? <= cut && f["dateSuperseded"].is_null() {
                let v = latest.entry(slot(f)).or_insert(start);
                *v = (*v).max(start);
            }
        }
        let mut pinned = BTreeSet::new();
        for f in &facts {
            let exception = window.keep_facts.iter().any(|r| {
                f["domain"] == r.domain
                    && f["fieldNumber"] == r.field
                    && text(&f["key"]) == r.key
                    && (r.values.is_empty() || r.values.iter().any(|v| v == text(&f["value"])))
            });
            let start = time(&f["dateStart"])?;
            let superseded = !f["dateSuperseded"].is_null() && time(&f["dateSuperseded"])? <= cut;
            if superseded || !exception && latest.get(&slot(f)).is_some_and(|s| *s > start) {
                out.facts.push(text(&f["id"]).into());
            } else {
                for k in ["eventId", "supersededBy"] {
                    if let Some(id) = f[k].as_str() {
                        pinned.insert(id.to_owned());
                    }
                }
            }
        }
        // Other application rows name an event only through a schema annotation.
        for e in &self.model.entities {
            if e.name == event.name || e.name == fact.name {
                continue;
            }
            for field in e.fields.iter().filter(|f| {
                self.policies
                    .get(&e.name)
                    .and_then(|p| p.fields.get(&f.name))
                    .is_some_and(|v| v["event"] == true)
            }) {
                let col = quote(&field.column());
                let hex = if self.db.dialect() == paydirt_model::Dialect::Postgres {
                    format!("encode({col},'hex')")
                } else {
                    format!("hex({col})")
                };
                let q = Query::new(format!(
                    "SELECT DISTINCT {hex} AS value FROM {} WHERE {col} IS NOT NULL",
                    quote(&e.table())
                ));
                for encoded in self.db.rows(q)? {
                    pinned.insert(
                        value::encoded(
                            &hex::decode(encoded)
                                .map_err(|_| Fault::internal("invalid event pointer"))?,
                        )
                        .as_str()
                        .unwrap()
                        .into(),
                    );
                }
            }
        }
        let mut parents = BTreeMap::new();
        let mut counts = BTreeMap::<String, usize>::new();
        let mut candidates = BTreeSet::new();
        for e in &events {
            let id = text(&e["id"]).to_owned();
            if let Some(parent) = e["causeId"].as_str() {
                parents.insert(id.clone(), parent.to_owned());
                *counts.entry(parent.into()).or_default() += 1;
            }
            if !pinned.contains(&id)
                && !window.keep_kinds.contains(text(&e["kind"]))
                && time(&e["dateOccurred"])? < cut
                && time(&e["dateCreated"])? < cut
            {
                candidates.insert(id);
            }
        }
        let mut queue: VecDeque<_> = candidates
            .iter()
            .filter(|id| !counts.contains_key(*id))
            .cloned()
            .collect();
        while let Some(id) = queue.pop_front() {
            if let Some(parent) = parents.get(&id)
                && let Some(n) = counts.get_mut(parent)
            {
                *n -= 1;
                if *n == 0 && candidates.contains(parent) {
                    queue.push_back(parent.clone())
                }
            }
            out.events.push(id);
        }
        if req["dry"] != true {
            for id in &out.facts {
                self.erase(&fact, &json!({"id":id}))?;
            }
            for id in &out.events {
                self.erase(&event, &json!({"id":id}))?;
            }
        }
        Ok(json!(out))
    }
}
