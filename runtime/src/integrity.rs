//! Payday v3 audit digests and v1 row seals. Each MAC binds a row to a particular audit line.
use crate::{Fault, Result};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use subtle::ConstantTimeEq;
type H = Hmac<Sha256>;
pub struct Line<'a> {
    pub id: [u8; 16],
    pub micros: i64,
    pub action: &'a str,
}
pub struct Keys {
    active: Vec<u8>,
    previous: BTreeMap<[u8; 8], Vec<u8>>,
    retired: BTreeSet<[u8; 8]>,
}
#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Matches,
    Differs,
    NoDigest,
    NoSeal,
    Missing,
    NotOnTrail,
    Forged,
    UnknownKey,
    Retired,
    Forgotten,
}
fn mac(k: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut h = H::new_from_slice(k).expect("HMAC key");
    for p in parts {
        h.update(p);
    }
    h.finalize().into_bytes().to_vec()
}
pub fn fingerprint(k: &[u8]) -> [u8; 8] {
    let mut h = Sha256::new();
    h.update(b"payday.trail.digest.key\0");
    h.update(k);
    h.finalize()[..8].try_into().unwrap()
}
fn row(k: &[u8], label: &[u8], object: &[u8; 16], fields: &[u32], wire: &[u8]) -> Vec<u8> {
    let mut h = H::new_from_slice(k).unwrap();
    h.update(label);
    h.update(object);
    h.update(&(fields.len() as u32).to_be_bytes());
    for n in fields {
        h.update(&n.to_be_bytes());
    }
    h.update(wire);
    h.finalize().into_bytes().to_vec()
}
fn binding(
    k: &[u8],
    label: &[u8],
    flags: &[u8],
    object: &[u8; 16],
    l: &Line<'_>,
    part: &[u8],
) -> Vec<u8> {
    mac(
        k,
        &[
            label,
            flags,
            &l.id,
            &l.micros.to_be_bytes(),
            &(l.action.len() as u32).to_be_bytes(),
            l.action.as_bytes(),
            object,
            part,
        ],
    )
}
impl Keys {
    pub fn new(
        active: Vec<u8>,
        previous: Vec<Vec<u8>>,
        retired: BTreeSet<[u8; 8]>,
    ) -> Result<Self> {
        if active.len() < 32 || previous.iter().any(|k| k.len() < 32) {
            return Err(Fault::invalid("audit keys need at least 32 bytes"));
        }
        let previous = previous
            .into_iter()
            .map(|k| (fingerprint(&k), k))
            .collect::<BTreeMap<_, _>>();
        if retired.contains(&fingerprint(&active)) || previous.keys().any(|k| retired.contains(k)) {
            return Err(Fault::invalid(
                "cannot retire a configured verification key",
            ));
        }
        Ok(Self {
            active,
            previous,
            retired,
        })
    }
    fn key(&self, fp: &[u8]) -> std::result::Result<&[u8], Verdict> {
        let fp = <&[u8; 8]>::try_from(fp).map_err(|_| Verdict::Forged)?;
        if fp == &fingerprint(&self.active) {
            Ok(&self.active)
        } else if let Some(k) = self.previous.get(fp) {
            Ok(k)
        } else if self.retired.contains(fp) {
            Err(Verdict::Retired)
        } else {
            Err(Verdict::UnknownKey)
        }
    }
    pub fn sign(
        &self,
        object: &[u8; 16],
        line: &Line<'_>,
        image: Option<(&[u32], &[u8])>,
        personal: bool,
    ) -> Vec<u8> {
        let flags = u8::from(personal && image.is_some());
        let mut part = Vec::new();
        if let Some((fields, wire)) = image {
            part.extend(fingerprint(&self.active));
            part.extend(row(
                &self.active,
                b"payday.trail.digest\0",
                object,
                fields,
                wire,
            ));
        }
        self.bind(object, line, flags, &part)
    }
    fn bind(&self, object: &[u8; 16], line: &Line<'_>, flags: u8, part: &[u8]) -> Vec<u8> {
        let mut sum = vec![3, flags];
        sum.extend(fingerprint(&self.active));
        sum.extend(binding(
            &self.active,
            b"payday.trail.line.3\0",
            &[flags],
            object,
            line,
            part,
        ));
        sum.extend(part);
        sum
    }
    pub fn seal(&self, object: &[u8; 16], line: &Line<'_>, fields: &[u32], wire: &[u8]) -> Vec<u8> {
        let k = mac(&self.active, &[b"payday.trail.seal.key\0"]);
        let fp = fingerprint(&self.active);
        let mut part = fp.to_vec();
        part.extend(row(&k, b"payday.trail.seal.row\0", object, fields, wire));
        let mut sum = vec![1, 0];
        sum.extend(fp);
        sum.extend(binding(
            &k,
            b"payday.trail.seal.line\0",
            b"",
            object,
            line,
            &part,
        ));
        sum.extend(part);
        sum
    }
    pub fn forgotten(&self, object: &[u8; 16]) -> Vec<u8> {
        let k = mac(&self.active, &[b"payday.trail.seal.key\0"]);
        let mut sum = vec![1, 1];
        sum.extend(fingerprint(&self.active));
        sum.extend(mac(&k, &[b"payday.trail.seal.forgotten\0", object]));
        sum
    }
    pub fn check(
        &self,
        sum: &[u8],
        object: &[u8; 16],
        l: &Line<'_>,
        image: Option<(&[u32], &[u8])>,
        seal: bool,
    ) -> Verdict {
        if !matches!(sum.len(), 42 | 82) || sum[0] != if seal { 1 } else { 3 } || sum[1] > 1 {
            return Verdict::Forged;
        }
        let k = match self.key(&sum[2..10]) {
            Ok(k) => k,
            Err(v) => return v,
        };
        let sk = mac(k, &[b"payday.trail.seal.key\0"]);
        if seal && sum[1] == 1 {
            return if sum.len() == 42
                && bool::from(
                    sum[10..].ct_eq(&mac(&sk, &[b"payday.trail.seal.forgotten\0", object])),
                ) {
                Verdict::Forgotten
            } else {
                Verdict::Forged
            };
        }
        let part = &sum[42..];
        let b = if seal {
            binding(&sk, b"payday.trail.seal.line\0", b"", object, l, part)
        } else {
            binding(k, b"payday.trail.line.3\0", &sum[1..2], object, l, part)
        };
        if !bool::from(sum[10..42].ct_eq(&b)) {
            return Verdict::Forged;
        }
        let Some((fields, wire)) = image else {
            return if part.is_empty() {
                Verdict::Matches
            } else {
                Verdict::Differs
            };
        };
        if part.is_empty() {
            return Verdict::Differs;
        }
        let k = match self.key(&part[..8]) {
            Ok(k) => k,
            Err(v) => return v,
        };
        let expected = if seal {
            row(
                &mac(k, &[b"payday.trail.seal.key\0"]),
                b"payday.trail.seal.row\0",
                object,
                fields,
                wire,
            )
        } else {
            row(k, b"payday.trail.digest\0", object, fields, wire)
        };
        if bool::from(part[8..].ct_eq(&expected)) {
            Verdict::Matches
        } else {
            Verdict::Differs
        }
    }
}

use crate::{
    Store, Transaction,
    driver::{Bind, Query},
    value,
};
use paydirt_model::{Entity, quote};
use serde_json::{Value, json};
impl Store {
    pub fn with_audit_keys(mut self, keys: Keys) -> Result<Self> {
        self.trail
            .as_mut()
            .ok_or_else(|| Fault::invalid("configure trail descriptors before audit keys"))?
            .keys = Some(keys);
        Ok(self)
    }
}
impl crate::audit_content::Recorder {
    fn covered_wire(&self, name: &str, row: &Value, fields: &[u32]) -> Result<Vec<u8>> {
        let mut m = self.message(name, row.clone())?;
        let desc = m.descriptor().clone();
        for f in desc.fields() {
            if !fields.contains(&f.number()) {
                m.clear_field(&f)
            }
        }
        Ok(m.encode_to_vec())
    }
}
fn id(v: &Value) -> Result<[u8; 16]> {
    value::bytes(v)?
        .try_into()
        .map_err(|_| Fault::internal("invalid audit object ID"))
}
impl Transaction<'_> {
    pub(crate) fn sign_write(
        &mut self,
        e: &Entity,
        after: Option<&Value>,
        audit: &mut Value,
    ) -> Result<()> {
        let Some(rec) = self.trail else { return Ok(()) };
        let Some(keys) = &rec.keys else { return Ok(()) };
        let audit_domain =
            self.policies
                .values()
                .find(|p| p.entity["own"] == "OWN_AUDIT")
                .and_then(|p| p.entity["domain"].as_u64())
                .ok_or_else(|| Fault::internal("missing Audit domain"))? as u8;
        let line_id = (self.inputs.mint)(audit_domain);
        let now = (self.inputs.clock)();
        audit["id"] = value::encoded(&line_id);
        audit["dateCreated"] = json!(now.to_rfc3339_opts(chrono::SecondsFormat::Micros, true));
        let action = audit["action"].as_str().unwrap().to_owned();
        let line = Line {
            id: line_id,
            micros: now.timestamp_micros(),
            action: &action,
        };
        let object = id(&audit["objectId"])?;
        let desc = rec
            .pool
            .get_message_by_name(&e.name)
            .ok_or_else(|| Fault::internal("missing audit descriptor"))?;
        let policy = &self.policies[&e.name];
        let mut all: Vec<_> = desc.fields().map(|f| f.number()).collect();
        all.sort();
        let personal: BTreeSet<_> = desc
            .fields()
            .filter(|f| {
                policy
                    .fields
                    .get(f.name())
                    .is_some_and(|v| v["personal"] == true)
            })
            .map(|f| f.number())
            .collect();
        let covered: Vec<_> = all
            .iter()
            .copied()
            .filter(|n| !personal.contains(n))
            .collect();
        let wire = after
            .map(|r| rec.covered_wire(&e.name, r, &covered))
            .transpose()?;
        let digest = keys.sign(
            &object,
            &line,
            wire.as_ref().map(|w| (covered.as_slice(), w.as_slice())),
            !personal.is_empty(),
        );
        let seal = if !personal.is_empty() {
            after
                .map(|r| {
                    rec.covered_wire(&e.name, r, &all)
                        .map(|wire| keys.seal(&object, &line, &all, &wire))
                })
                .transpose()?
        } else {
            None
        };
        audit["digest"] = value::encoded(&digest);
        audit["digestFields"] = json!(if after.is_some() { covered } else { vec![] });
        if !personal.is_empty() {
            self.save_seal(&object, &audit["tenantId"], seal.as_deref(), &all)?;
        }
        Ok(())
    }
    fn save_seal(
        &mut self,
        object: &[u8; 16],
        tenant: &Value,
        sum: Option<&[u8]>,
        fields: &[u32],
    ) -> Result<()> {
        let name = self
            .policies
            .iter()
            .find(|(_, p)| p.entity["own"] == "OWN_ROW_SEAL")
            .map(|(n, _)| n)
            .ok_or_else(|| Fault::internal("missing RowSeal schema"))?;
        let table = quote(&self.model.entity(name).map_err(Fault::internal)?.table());
        let mut q = Query::new("");
        let d = self.db.dialect();
        let id = q.param(Bind::Bytes(object.to_vec()), d);
        q.sql = if let Some(sum) = sum {
            let t = q.param(Bind::Bytes(value::bytes(tenant)?), d);
            let s = q.param(Bind::Bytes(sum.to_vec()), d);
            let f = q.param(Bind::Text(json!(fields).to_string()), d);
            format!(
                "INSERT INTO {table}(f1,f2,f8,f9) VALUES ({id},{t},{s},{f}) ON CONFLICT(f1) DO UPDATE SET f2=excluded.f2,f8=excluded.f8,f9=excluded.f9"
            )
        } else {
            format!("DELETE FROM {table} WHERE f1={id}")
        };
        self.db.exec(q)?;
        Ok(())
    }
}

impl Transaction<'_> {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn forget_seal(&mut self, object: &[u8; 16]) -> Result<()> {
        let Some(name) = self
            .policies
            .iter()
            .find(|(_, p)| p.entity["own"] == "OWN_ROW_SEAL")
            .map(|(n, _)| n)
        else {
            return Ok(());
        };
        let table = quote(&self.model.entity(name).map_err(Fault::internal)?.table());
        let d = self.db.dialect();
        let mut q = Query::new("");
        let p = q.param(Bind::Bytes(object.to_vec()), d);
        q.sql = format!("SELECT '1' AS value FROM {table} WHERE f1={p}");
        if self.db.rows(q)?.is_empty() {
            return Ok(());
        }
        let keys = self.trail.and_then(|t| t.keys.as_ref()).ok_or_else(|| {
            Fault::precondition("forgetting an existing row seal requires the audit key")
        })?;
        let mut q = Query::new("");
        let p = q.param(Bind::Bytes(object.to_vec()), d);
        let s = q.param(Bind::Bytes(keys.forgotten(object)), d);
        q.sql = format!("UPDATE {table} SET f8={s},f9='[]' WHERE f1={p}");
        self.db.exec(q)?;
        Ok(())
    }
}

#[derive(serde::Serialize, Debug)]
pub struct Finding {
    pub digest: Verdict,
    pub seal: Option<Verdict>,
}

#[derive(serde::Serialize, Debug, Default)]
pub struct KindReport {
    pub checked: usize,
    pub archived: usize,
    pub findings: BTreeMap<String, Finding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}
pub(crate) fn order(row: &Value) -> Result<(chrono::DateTime<chrono::Utc>, [u8; 16])> {
    let time = row["dateCreated"]
        .as_str()
        .ok_or_else(|| Fault::internal("missing audit date"))?
        .parse()
        .map_err(|_| Fault::internal("invalid audit date"))?;
    Ok((time, id(&row["id"])?))
}
type Heads = BTreeMap<[u8; 16], (Value, bool)>;
fn consider(heads: &mut Heads, row: Value, archived: bool) -> Result<()> {
    let object = id(&row["objectId"])?;
    if let Some((existing, _)) = heads.get(&object)
        && order(existing)? >= order(&row)?
    {
        return Ok(());
    }
    heads.insert(object, (row, archived));
    Ok(())
}
impl Transaction<'_> {
    fn owned_kind(&self, kind: &str) -> Result<&Entity> {
        let (name, _) = self
            .policies
            .iter()
            .find(|(_, p)| p.entity["own"] == kind)
            .ok_or_else(|| Fault::precondition(format!("missing {kind} schema")))?;
        self.model.entity(name).map_err(Fault::internal)
    }
    fn online_heads(&mut self, name: &str, object: Option<[u8; 16]>) -> Result<Heads> {
        let audit = self.owned_kind("OWN_AUDIT")?.clone();
        let domain = self
            .policies
            .get(name)
            .and_then(|p| p.entity["domain"].as_i64())
            .ok_or_else(|| Fault::invalid("expected an entity with a domain"))?;
        let mut heads = Heads::new();
        let mut cursor = None;
        loop {
            let mut q = Query::new("");
            let dialect = self.db.dialect();
            let bound = q.param(Bind::Big(domain), dialect);
            let mut predicate = format!("r.f19={bound}");
            if let Some(id) = object {
                let key = q.param(Bind::Bytes(id.to_vec()), dialect);
                predicate.push_str(&format!(" AND r.f11={key}"));
            }
            if let Some((stamp, key)) = &cursor {
                let at = q.param(
                    value::bind(audit.field("date_created").map_err(Fault::internal)?, stamp)?,
                    dialect,
                );
                let id = q.param(
                    value::bind(audit.key().map_err(Fault::internal)?, key)?,
                    dialect,
                );
                predicate.push_str(&format!(" AND (r.f15,r.f1)>({at},{id})"));
            }
            let direction = if object.is_some() { "DESC" } else { "ASC" };
            let limit = if object.is_some() { 1 } else { 256 };
            q.sql = format!(
                "SELECT {} AS value FROM {} r WHERE {predicate} ORDER BY r.f15 {direction},r.f1 {direction} LIMIT {limit}",
                self.project_sql(&audit)?,
                quote(&audit.table())
            );
            let rows = self.db.rows(q)?;
            let count = rows.len();
            for raw in rows {
                let row = self.decode_row(&audit, &raw)?;
                cursor = Some((row["dateCreated"].clone(), row["id"].clone()));
                consider(&mut heads, row, false)?;
            }
            if object.is_some() || count < limit {
                break;
            }
        }
        Ok(heads)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn heads(
        &mut self,
        name: &str,
        object: Option<[u8; 16]>,
        archive: Option<&crate::archive_files::Archive>,
    ) -> Result<Heads> {
        let mut heads = self.online_heads(name, object)?;
        if let Some(files) = archive {
            for row in self.archived_digests(files, name, object)? {
                consider(&mut heads, row, true)?;
            }
        }
        Ok(heads)
    }
    fn inspect_sum(&mut self, e: &Entity, object: [u8; 16], audit: &Value) -> Result<Finding> {
        let rec = self
            .trail
            .ok_or_else(|| Fault::precondition("trail recorder required"))?;
        let keys = rec
            .keys
            .as_ref()
            .ok_or_else(|| Fault::precondition("verification keys required"))?;
        let (at, key) = order(audit)?;
        let line = Line {
            id: key,
            micros: at.timestamp_micros(),
            action: audit["action"]
                .as_str()
                .ok_or_else(|| Fault::internal("missing audit action"))?,
        };
        let sum = audit["digest"]
            .as_str()
            .map(|_| value::bytes(&audit["digest"]))
            .transpose()?
            .unwrap_or_default();
        if sum.is_empty() {
            return Ok(Finding {
                digest: Verdict::NoDigest,
                seal: None,
            });
        }
        let current = match self.lookup(
            e,
            &json!({&e.key().map_err(Fault::internal)?.json:value::encoded(&object)}),
            false,
        ) {
            Ok(r) => Some(r),
            Err(v) if v.code == "not_found" => None,
            Err(e) => return Err(e),
        };
        let check = |sum: &[u8], fields: &Value, seal: bool| -> Result<Verdict> {
            let covered: Vec<u32> = serde_json::from_value(fields.clone())
                .map_err(|e| Fault::precondition(e.to_string()))?;
            let wire = current
                .as_ref()
                .map(|row| rec.covered_wire(&e.name, row, &covered))
                .transpose()?;
            let verdict = keys.check(
                sum,
                &object,
                &line,
                wire.as_ref().map(|w| (covered.as_slice(), w.as_slice())),
                seal,
            );
            Ok(if verdict == Verdict::Differs && current.is_none() {
                Verdict::Missing
            } else {
                verdict
            })
        };
        let digest = check(&sum, &audit["digestFields"], false)?;
        let seal = if sum.get(1) == Some(&1) && (current.is_some() || sum.len() > 42) {
            let table = self.owned_kind("OWN_ROW_SEAL")?.clone();
            let mut q = Query::new("");
            let key = q.param(Bind::Bytes(object.to_vec()), self.db.dialect());
            q.sql = format!(
                "SELECT {} AS value FROM {} r WHERE r.f1={key}",
                self.project_sql(&table)?,
                quote(&table.table())
            );
            Some(if let Some(row) = self.db.rows(q)?.pop() {
                let s = self.decode_row(&table, &row)?;
                check(&value::bytes(&s["sum"])?, &s["fields"], true)?
            } else {
                Verdict::NoSeal
            })
        } else {
            None
        };
        Ok(Finding { digest, seal })
    }
}
impl Store {
    pub fn verify_object(&mut self, name: &str, object: [u8; 16]) -> Result<Finding> {
        self.transaction(&crate::Scope::Internal, |tx| {
            let e = tx.model.entity(name).map_err(Fault::invalid)?;
            let (head, _) = tx
                .online_heads(name, Some(object))?
                .remove(&object)
                .ok_or_else(Fault::missing)?;
            tx.inspect_sum(e, object, &head)
        })
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn verify_object_with_archive(
        &mut self,
        name: &str,
        object: [u8; 16],
        archive: Option<&crate::archive_files::Archive>,
    ) -> Result<Finding> {
        self.transaction(&crate::Scope::Internal, |tx| {
            let e = tx.model.entity(name).map_err(Fault::invalid)?;
            let (head, _) = tx
                .heads(name, Some(object), archive)?
                .remove(&object)
                .ok_or_else(Fault::missing)?;
            tx.inspect_sum(e, object, &head)
        })
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn check_kind(
        &mut self,
        name: &str,
        archive: Option<&crate::archive_files::Archive>,
    ) -> Result<KindReport> {
        self.transaction(&crate::Scope::Internal, |tx| {
            let e = tx.model.entity(name).map_err(Fault::invalid)?;
            let policy = tx
                .policies
                .get(name)
                .ok_or_else(|| Fault::invalid("unknown audit kind"))?;
            let mut report = KindReport::default();
            let key = e.key().map_err(Fault::invalid)?;
            if let Some(why) = policy.entity["trail"].get("off") {
                report.skipped = Some(
                    why.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| why.to_string()),
                );
                return Ok(report);
            }
            if key.kind != "UUID" || policy.entity["domain"].as_u64().is_none() {
                report.skipped = Some("entity does not have a Payday identifier".into());
                return Ok(report);
            }
            let mut heads = tx.heads(name, None, archive)?;
            let mut cursor = None;
            loop {
                let mut q = Query::new("");
                let mut filters = tx.wall(e, &mut q)?;
                let column = format!("r.{}", quote(&key.column()));
                if let Some(id) = cursor {
                    let bound = q.param(Bind::Bytes(id), tx.db.dialect());
                    filters.push(format!("{column}>{bound}"));
                }
                if filters.is_empty() {
                    filters.push("TRUE".into());
                }
                q.sql = format!(
                    "SELECT {} AS value FROM {} r WHERE {} ORDER BY {column} LIMIT 256",
                    tx.project_sql(e)?,
                    quote(&e.table()),
                    filters.join(" AND ")
                );
                let rows = tx.db.rows(q)?;
                let count = rows.len();
                cursor = None;
                for raw in rows {
                    let row = tx.decode_row(e, &raw)?;
                    let object = id(&row[&key.json])?;
                    heads.entry(object).or_insert((Value::Null, false));
                    cursor = Some(object.to_vec());
                }
                if count < 256 {
                    break;
                }
            }
            for (object, (row, archived)) in heads {
                report.checked += 1;
                report.archived += usize::from(archived);
                let finding = if row.is_null() {
                    Finding {
                        digest: Verdict::NotOnTrail,
                        seal: None,
                    }
                } else {
                    tx.inspect_sum(e, object, &row)?
                };
                if !matches!(finding.digest, Verdict::Matches | Verdict::Retired)
                    || finding.seal.is_some_and(|s| {
                        !matches!(s, Verdict::Matches | Verdict::Forgotten | Verdict::Retired)
                    })
                {
                    report.findings.insert(
                        paydirt::format_id(&object).expect("sixteen-byte id"),
                        finding,
                    );
                }
            }
            Ok(report)
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Transaction<'_> {
    /// A host explicitly accepts the current value (or absence) as a new signed act.
    pub fn record_objects(
        &mut self,
        name: &str,
        objects: &[[u8; 16]],
        action: &str,
        erased: bool,
    ) -> Result<usize> {
        self.record_objects_with_archive(name, objects, action, erased, None)
    }
    pub fn record_objects_with_archive(
        &mut self,
        name: &str,
        objects: &[[u8; 16]],
        action: &str,
        erased: bool,
        archive: Option<&crate::archive_files::Archive>,
    ) -> Result<usize> {
        let result = self.record_current_objects(name, objects, action, erased, archive);
        if let Err(err) = &result {
            self.failed = Some(err.clone());
        }
        result
    }
    fn record_current_objects(
        &mut self,
        name: &str,
        objects: &[[u8; 16]],
        action: &str,
        erased: bool,
        archive: Option<&crate::archive_files::Archive>,
    ) -> Result<usize> {
        if !matches!(self.scope, crate::Scope::Internal) {
            return Err(Fault::denied("record requires a host transaction"));
        }
        if action.trim().is_empty() {
            return Err(Fault::invalid("the act being recorded must be named"));
        }
        let entity = self.model.entity(name).map_err(Fault::invalid)?;
        let policy = self
            .policies
            .get(name)
            .ok_or_else(|| Fault::invalid("unknown audit kind"))?;
        if policy.entity["trail"].get("off").is_some() {
            return Ok(0);
        }
        let domain = policy.entity["domain"]
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .ok_or_else(|| Fault::invalid("expected a domain"))?;
        if self.trail.and_then(|r| r.keys.as_ref()).is_none() {
            return Err(Fault::precondition("record requires digest keys"));
        }
        let audit = self.owned_kind("OWN_AUDIT")?.clone();
        let objects: BTreeSet<_> = objects.iter().copied().collect();
        for &object in &objects {
            if !paydirt::has_domain(&object, domain) {
                return Err(Fault::invalid("identifier belongs to another kind"));
            }
            let row = match self.lookup(
                entity,
                &json!({&entity.key().map_err(Fault::internal)?.json:value::encoded(&object)}),
                true,
            ) {
                Ok(_) if erased => return Err(Fault::precondition("object still exists")),
                Ok(row) => Some(row),
                Err(f) if f.code == "not_found" && erased => None,
                Err(f) => return Err(f),
            };
            let tenant = match &row {
                Some(r) => self.object_tenant(entity, r)?,
                None => {
                    let (latest, _) = self
                        .heads(name, Some(object), archive)?
                        .remove(&object)
                        .ok_or_else(|| Fault::precondition("no prior audit for absent object"))?;
                    latest["tenantId"].clone()
                }
            };
            let mut line = json!({"tenantId":tenant,"objectId":value::encoded(&object),"action":action,"domain":domain,
                "actorTenantId":value::encoded(&[0;16]),"actorId":value::encoded(&[0;16]),"traceId":value::encoded(&[0;16])});
            self.sign_write(entity, row.as_ref(), &mut line)?;
            self.insert(&audit, &line)?;
        }
        Ok(objects.len())
    }
}
