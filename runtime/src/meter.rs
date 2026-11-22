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
        self.measure_at(tenant, Utc::now())
    }
    pub fn measure_at(&mut self, tenant: &[u8], at: chrono::DateTime<Utc>) -> Result<Measurements> {
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
        let time = at.to_rfc3339_opts(SecondsFormat::Micros, true);
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

/// First epoch-aligned UTC measurement boundary strictly after now.
pub fn next_boundary(
    now: chrono::DateTime<Utc>,
    every: std::time::Duration,
) -> Result<chrono::DateTime<Utc>> {
    let ns = i64::try_from(every.as_nanos())
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| Fault::invalid("invalid meter interval"))?;
    let t = now
        .timestamp_nanos_opt()
        .ok_or_else(|| Fault::invalid("clock outside scheduling range"))?;
    let n = t
        .div_euclid(ns)
        .checked_add(1)
        .and_then(|n| n.checked_mul(ns))
        .ok_or_else(|| Fault::invalid("meter boundary overflow"))?;
    Ok(chrono::DateTime::from_timestamp_nanos(n))
}
impl Store {
    pub fn meter_tenants(&mut self, after: Option<&[u8]>, limit: usize) -> Result<Vec<Vec<u8>>> {
        if !(1..=1000).contains(&limit) {
            return Err(Fault::invalid("invalid tenant page size"));
        }
        self.transaction(&Scope::Internal, |tx| {
            let entity = tx.owned_entity("OWN_TENANT")?;
            let id = quote(&entity.key().map_err(Fault::internal)?.column());
            let mut q = Query::new("");
            let dialect = tx.db.dialect();
            let mut parts = Vec::new();
            if let Some(token) = after {
                if token.len() != 16 {
                    return Err(Fault::invalid("invalid tenant cursor"));
                }
                let bind = q.param(crate::driver::Bind::Bytes(token.to_vec()), dialect);
                parts.push(format!("{id}>{bind}"));
            }
            if let Some(f) = entity.erased() {
                parts.push(format!("{} IS NULL", quote(&f.column())));
            }
            let filter = if parts.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", parts.join(" AND "))
            };
            let select = if dialect == paydirt_model::Dialect::Postgres {
                format!("encode({id},'hex')")
            } else {
                format!("hex({id})")
            };
            q.sql = format!(
                "SELECT {select} AS value FROM {} {filter} ORDER BY {id} LIMIT {limit}",
                quote(&entity.table())
            );
            tx.db
                .rows(q)?
                .iter()
                .map(|s| hex::decode(s).map_err(|_| Fault::internal("malformed stored tenant ID")))
                .collect()
        })
    }
}

/// One pass keeps the cursor and undelivered values across retries up to its next boundary.
pub struct Pass {
    at: chrono::DateTime<Utc>,
    after: Option<Vec<u8>>,
    listed: bool,
    retry: BTreeSet<Vec<u8>>,
    owed: std::collections::BTreeMap<Vec<u8>, Vec<Reading>>,
    accepted: std::collections::BTreeMap<Vec<u8>, BTreeSet<String>>,
}
impl Pass {
    pub fn new(at: chrono::DateTime<Utc>) -> Self {
        Self {
            at,
            after: None,
            listed: false,
            retry: Default::default(),
            owed: Default::default(),
            accepted: Default::default(),
        }
    }
    pub fn done(&self) -> bool {
        self.listed && self.retry.is_empty() && self.owed.is_empty()
    }
    pub fn run(
        &mut self,
        store: &mut Store,
        send: impl FnMut(&[Reading]) -> Result<()>,
    ) -> Vec<String> {
        self.run_with(
            store,
            |s, after, size| s.meter_tenants(after, size),
            |s, id, at| s.measure_at(id, at),
            send,
        )
    }
    /// The source must page strictly after the supplied ID. The caller may supply its own tenant and meter implementations.
    pub fn run_with<S>(
        &mut self,
        source: &mut S,
        mut page: impl FnMut(&mut S, Option<&[u8]>, usize) -> Result<Vec<Vec<u8>>>,
        mut measure: impl FnMut(&mut S, &[u8], chrono::DateTime<Utc>) -> Result<Measurements>,
        mut send: impl FnMut(&[Reading]) -> Result<()>,
    ) -> Vec<String> {
        const SIZE: usize = 100;
        let mut errors = Vec::new();
        let mut tenants = std::mem::take(&mut self.retry);
        let mut delivery = std::mem::take(&mut self.owed);
        loop {
            for id in std::mem::take(&mut tenants) {
                match measure(source, &id, self.at) {
                    Err(e) => {
                        errors.push(e.to_string());
                        self.retry.insert(id);
                    }
                    Ok(batch) => {
                        if !batch.errors.is_empty() {
                            errors.extend(batch.errors);
                            self.retry.insert(id.clone());
                        }
                        let waiting = delivery.entry(id.clone()).or_default();
                        for row in batch.readings {
                            if !self
                                .accepted
                                .get(&id)
                                .is_some_and(|s| s.contains(&row.meter))
                                && !waiting.iter().any(|v| v.meter == row.meter)
                            {
                                waiting.push(row);
                            }
                        }
                    }
                }
            }
            for (id, rows) in std::mem::take(&mut delivery) {
                if rows.is_empty() {
                    continue;
                }
                match send(&rows) {
                    Ok(()) if self.retry.contains(&id) => {
                        self.accepted
                            .entry(id)
                            .or_default()
                            .extend(rows.into_iter().map(|r| r.meter));
                    }
                    Ok(()) => {
                        self.accepted.remove(&id);
                    }
                    Err(e) => {
                        errors.push(e.to_string());
                        self.owed.insert(id, rows);
                    }
                }
            }
            if self.listed {
                break;
            }
            match page(source, self.after.as_deref(), SIZE) {
                Err(e) => {
                    errors.push(e.to_string());
                    break;
                }
                Ok(ids) => {
                    if ids.len() > SIZE
                        || ids
                            .first()
                            .zip(self.after.as_ref())
                            .is_some_and(|(a, b)| a <= b)
                        || ids.windows(2).any(|v| v[0] >= v[1])
                    {
                        errors.push("invalid meter tenant page order".into());
                        break;
                    }
                    self.listed = ids.len() < SIZE;
                    if let Some(last) = ids.last() {
                        self.after = Some(last.clone());
                    }
                    tenants.extend(ids);
                }
            }
        }
        errors
    }
}

/// Schedule one reader on startup and at epoch-aligned boundaries. An application
/// can use Pass::run_with here to add its own meter, tenant source or sink.
#[cfg(feature = "workers")]
pub async fn periodic(
    stop: paydirt::spin::Stop,
    every: std::time::Duration,
    first_retry: std::time::Duration,
    mut run: impl FnMut(&mut Pass) -> Vec<String>,
) -> Result<()> {
    use std::time::Duration;
    if first_retry.is_zero() {
        return Err(Fault::invalid("meter retry must be positive"));
    }
    next_boundary(Utc::now(), every)?;
    while !stop.stopped() {
        let mut pass = Pass::new(Utc::now());
        for error in run(&mut pass) {
            eprintln!("meter: {error}");
        }
        let next = next_boundary(Utc::now(), every)?;
        let mut delay = first_retry;
        loop {
            let remaining = (next - Utc::now()).to_std().unwrap_or_default();
            if remaining.is_zero() {
                break;
            }
            let retry = !pass.done() && delay < remaining;
            let wait = if retry { delay } else { remaining };
            tokio::select! {_=stop.cancelled()=>return Ok(()),_=tokio::time::sleep(wait)=>{}}
            if !retry || Utc::now() >= next {
                break;
            }
            for error in run(&mut pass) {
                eprintln!("meter: {error}");
            }
            delay = delay.saturating_mul(2).min(Duration::from_secs(3600));
        }
        if !pass.done() {
            eprintln!(
                "meter: unfinished pass at {} reached its next boundary",
                pass.at
            );
        }
    }
    Ok(())
}

/// Store independent readings atomically. Their identity includes the original
/// measurement timestamp so retrying partial delivery is idempotent.
#[cfg(feature = "workers")]
pub fn directory_sink(directory: &std::path::Path, rows: &[Reading]) -> Result<()> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    let io = |e: std::io::Error| Fault::unavailable(e.to_string());
    std::fs::create_dir_all(directory).map_err(io)?;
    for row in rows {
        let key = serde_json::to_vec(&(&row.tenant, &row.meter, &row.measured_at))
            .map_err(|e| Fault::internal(e.to_string()))?;
        let path = directory.join(format!("{}.json", hex::encode(Sha256::digest(key))));
        let mut temp = tempfile::NamedTempFile::new_in(directory).map_err(io)?;
        serde_json::to_writer(temp.as_file_mut(), row)
            .map_err(|e| Fault::unavailable(e.to_string()))?;
        temp.as_file_mut().flush().map_err(io)?;
        temp.as_file().sync_all().map_err(io)?;
        match temp.persist_noclobber(path) {
            Ok(_) => {}
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io(e.error)),
        }
    }
    Ok(())
}
