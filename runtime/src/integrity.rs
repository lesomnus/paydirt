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
impl Store {
    pub fn verify_object(&mut self, name: &str, object: [u8; 16]) -> Result<Finding> {
        self.transaction(&crate::Scope::Internal,|tx|{
            let rec=tx.trail.ok_or_else(||Fault::precondition("trail recorder required"))?;
            let keys=rec.keys.as_ref().ok_or_else(||Fault::precondition("verification keys required"))?;
            let e=tx.model.entity(name).map_err(Fault::invalid)?;
            let owned=|kind:&str|->Result<&Entity>{
                let name=tx.policies.iter().find(|(_,p)|p.entity["own"]==kind).map(|(n,_)|n).ok_or_else(||Fault::internal("missing verification schema"))?;
                tx.model.entity(name).map_err(Fault::internal)
            };
            let audit=owned("OWN_AUDIT")?;let seal=owned("OWN_ROW_SEAL")?;
            let mut q=Query::new("");let p=q.param(Bind::Bytes(object.to_vec()),tx.db.dialect());q.sql=format!("SELECT {} AS value FROM {} r WHERE r.f11={p} ORDER BY r.f15 DESC,r.f1 DESC LIMIT 1",tx.project_sql(audit)?,quote(&audit.table()));
            let raw=tx.db.rows(q)?.pop().ok_or_else(Fault::missing)?;let a=tx.decode_row(audit,&raw)?;
            let line=Line{id:id(&a["id"])?,micros:a["dateCreated"].as_str().ok_or_else(||Fault::internal("missing line time"))?.parse::<chrono::DateTime<chrono::Utc>>().map_err(|e|Fault::internal(e.to_string()))?.timestamp_micros(),action:a["action"].as_str().ok_or_else(||Fault::internal("missing line action"))?};
            let sum=value::bytes(&a["digest"])?;
            let current=match tx.lookup(e,&json!({&e.key().map_err(Fault::internal)?.json:value::encoded(&object)}),false){Ok(r)=>Some(r),Err(v)if v.code=="not_found"=>None,Err(e)=>return Err(e)};
            let check=|sum:&[u8],fields:&Value,seal:bool|->Result<Verdict>{
                let covered:Vec<u32>=serde_json::from_value(fields.clone()).map_err(|e|Fault::precondition(e.to_string()))?;
                let wire=current.as_ref().map(|row|rec.covered_wire(name,row,&covered)).transpose()?;
                Ok(keys.check(sum,&object,&line,wire.as_ref().map(|w|(covered.as_slice(),w.as_slice())),seal))
            };
            let digest=check(&sum,&a["digestFields"],false)?;
            let seal_result=if sum.get(1)==Some(&1){
                let mut q=Query::new("");let p=q.param(Bind::Bytes(object.to_vec()),tx.db.dialect());q.sql=format!("SELECT {} AS value FROM {} r WHERE r.f1={p}",tx.project_sql(seal)?,quote(&seal.table()));
                Some(if let Some(row)=tx.db.rows(q)?.pop(){let s=tx.decode_row(seal,&row)?;check(&value::bytes(&s["sum"])?,&s["fields"],true)?}else{Verdict::Differs})
            }else{None};
            Ok(Finding{digest,seal:seal_result})
        })
    }
}
