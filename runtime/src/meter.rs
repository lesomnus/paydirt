//! Schema-backed measurements. Each successful reading retains its tenant, key, unit and timestamp.
use crate::{Fault, Result, Scope, Store, Transaction, driver::Query, value};
use chrono::{SecondsFormat, Utc};
use paydirt_model::{Entity, Field, quote};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub fn valid_key(key: &str) -> bool {
    key.len() <= 255
        && key.contains('.')
        && key.split('.').all(|w| {
            w.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                && w.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}
pub fn valid_unit(unit: &str) -> bool {
    (1..=63).contains(&unit.len()) && unit.bytes().all(|c| (b'!'..=b'~').contains(&c))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub tenant: Value,
    pub meter: String,
    pub measured_at: String,
    pub value: i64,
    pub unit: String,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Measurements {
    pub readings: Vec<Reading>,
    pub errors: Vec<String>,
}
impl Measurements {
    /// Hand over the successfully measured readings even when one other meter failed.
    pub fn deliver(&self, send: impl FnOnce(&[Reading]) -> Result<()>) -> Result<()> {
        let mut errs = self.errors.clone();
        if !self.readings.is_empty()
            && let Err(e) = send(&self.readings)
        {
            errs.push(e.to_string())
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(Fault::precondition(errs.join(", ")))
        }
    }
}
impl Store {
    pub fn measure(&mut self, tenant: &[u8]) -> Result<Measurements> {
        if !paydirt::has_domain(tenant, 1) {
            return Err(Fault::invalid("meter requires a tenant ID"));
        }
        let scope = Scope::Tenant(tenant.to_vec());
        let declared = self
            .policies
            .values()
            .find(|p| p.entity["own"] == "OWN_HOLDER")
            .and_then(|p| p.application["meters"].as_array())
            .cloned()
            .unwrap_or_default();
        let mut keys = BTreeSet::new();
        for spec in &declared {
            let key = spec["key"].as_str().unwrap_or("");
            let unit = spec["unit"].as_str().unwrap_or("");
            if !valid_key(key) || !valid_unit(unit) || !keys.insert(key) {
                return Err(Fault::invalid("invalid or duplicate meter declaration"));
            }
        }
        let time = Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true);
        let mut batch = Measurements::default();
        for spec in declared {
            let key = spec["key"].as_str().unwrap().to_owned();
            match self.transaction(&scope, |tx| tx.measured(&spec)) {
                Ok(n) => batch.readings.push(Reading {
                    tenant: value::encoded(tenant),
                    meter: key,
                    measured_at: time.clone(),
                    value: n,
                    unit: spec["unit"].as_str().unwrap().to_owned(),
                }),
                Err(e) => batch.errors.push(format!("{key}: {e}")),
            }
        }
        Ok(batch)
    }
}
impl Transaction<'_> {
    fn measured(&mut self, spec: &Value) -> Result<i64> {
        let (op, sum) = if let Some(count) = spec.get("count") {
            (count, false)
        } else if let Some(sum) = spec.get("sum") {
            (sum, true)
        } else {
            return Err(Fault::invalid("unknown meter measurement"));
        };
        let holder = self.owned_entity("OWN_HOLDER")?;
        let package = holder.name.rsplit_once('.').unwrap().0;
        let short = op["of"]
            .as_str()
            .ok_or_else(|| Fault::invalid("missing meter entity"))?;
        let name = if short.contains('.') {
            short.to_owned()
        } else {
            format!("{package}.{short}")
        };
        let e = self.model.entity(&name).map_err(Fault::invalid)?;
        if self.policies[&name].entity.get("global").is_some() {
            return Err(Fault::invalid("tenant meter cannot measure global rows"));
        }
        fn field<'a>(e: &'a Entity, decl: &Value) -> Result<&'a Field> {
            let name = decl["name"]
                .as_str()
                .ok_or_else(|| Fault::invalid("missing meter field"))?;
            let f = e.field(name).map_err(Fault::invalid)?;
            if Some(f.number as u64) != decl["number"].as_u64() {
                return Err(Fault::invalid("meter field number changed"));
            }
            Ok(f)
        }
        let d = self.db.dialect();
        let mut q = Query::new("");
        let mut parts = self.wall(e, &mut q)?;
        for predicate in op["where"].as_array().into_iter().flatten() {
            let f = field(e, &predicate["field"])?;
            let col = format!("r.{}", quote(&f.column()));
            if let Some(set) = predicate["set"].as_bool() {
                parts.push(format!("{col} IS {}NULL", if set { "NOT " } else { "" }));
                continue;
            }
            let (options, negated) = if let Some(i) = predicate["in"].as_array() {
                (i, false)
            } else if let Some(i) = predicate["notIn"].as_array() {
                (i, true)
            } else {
                return Err(Fault::invalid("missing meter predicate"));
            };
            if options.is_empty() {
                return Err(Fault::invalid("empty meter predicate"));
            }
            let mut vs = Vec::new();
            for raw in options {
                let s = raw
                    .as_str()
                    .ok_or_else(|| Fault::invalid("invalid meter value"))?;
                let v = match f.kind.as_str() {
                    "BOOL" => json!(
                        s.parse::<bool>()
                            .map_err(|_| Fault::invalid("invalid bool meter filter"))?
                    ),
                    "INT32" | "SINT32" | "SFIXED32" | "UINT32" | "FIXED32" => json!(
                        s.parse::<i64>()
                            .map_err(|_| Fault::invalid("invalid meter number"))?
                    ),
                    _ => json!(s),
                };
                vs.push(q.param(value::bind(f, &v)?, d));
            }
            parts.push(format!(
                "{col} {}IN ({})",
                if negated { "NOT " } else { "" },
                vs.join(",")
            ));
        }
        let expression = if sum {
            let f = field(e, &op["field"])?;
            if !matches!(
                f.kind.as_str(),
                "INT32"
                    | "SINT32"
                    | "SFIXED32"
                    | "UINT32"
                    | "FIXED32"
                    | "INT64"
                    | "SINT64"
                    | "SFIXED64"
                    | "UINT64"
                    | "FIXED64"
            ) {
                return Err(Fault::invalid("a meter sums integers"));
            }
            format!("COALESCE(SUM(CAST(r.{} AS NUMERIC)),0)", quote(&f.column()))
        } else {
            "COUNT(*)".into()
        };
        q.sql = format!(
            "SELECT CAST({expression} AS TEXT) AS value FROM {} r WHERE {}",
            quote(&e.table()),
            parts.join(" AND ")
        );
        self.db
            .rows(q)?
            .first()
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| Fault::precondition("measurement outside int64 range"))
    }
}
