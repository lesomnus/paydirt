//! The generated application's Archived table and signed manifest transitions.
use crate::{
    Fault, Result, Scope, Transaction,
    attestation::{Notary, Signature, manifest_statement},
    driver::{Bind, Query},
    value,
};
use paydirt_model::{Entity, quote};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn bad(v: impl ToString) -> Fault {
    Fault::precondition(v.to_string())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Adding,
    Present,
    Erasing,
    Erased,
}
impl State {
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Adding => "adding",
            Self::Present => "present",
            Self::Erasing => "erasing",
            Self::Erased => "erased",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Record {
    pub id: [u8; 16],
    pub namespace: String,
    pub digest: String,
    pub intent: String,
    pub state: State,
    pub labels: BTreeMap<String, String>,
    pub since: u32,
    pub gone: u32,
    pub signature: Option<Signature>,
}
impl Record {
    pub fn statement(&self) -> Vec<u8> {
        manifest_statement(
            &paydirt::format_id(&self.id).unwrap(),
            &self.namespace,
            &self.digest,
            &self.intent,
            self.state.name(),
            &self.labels,
        )
    }
    pub fn verify(&self, notary: Option<&Notary>) -> Result<()> {
        match (notary, &self.signature) {
            (Some(n), Some(s)) => n.verify(&self.statement(), s),
            (None, None) => Ok(()),
            _ => Err(bad("manifest signature or verification key missing")),
        }
    }
    fn decode(row: &Value) -> Result<Self> {
        let text = |field: &str| {
            row[field]
                .as_str()
                .ok_or_else(|| bad(format!("invalid manifest {field}")))
                .map(str::to_owned)
        };
        let uint = |field: &str| row[field].as_u64().unwrap_or(0).try_into().map_err(bad);
        let signature = text("signature")?;
        let signer = text("signedBy")?;
        if signature.is_empty() != signer.is_empty() {
            return Err(bad("incomplete manifest signature"));
        }
        Ok(Self {
            id: value::bytes(&row["id"])?
                .try_into()
                .map_err(|_| bad("invalid manifest id"))?,
            namespace: text("namespace")?,
            digest: text("digest")?,
            intent: text("intent")?,
            state: serde_json::from_value(row["state"].clone()).map_err(bad)?,
            labels: serde_json::from_value(row["labels"].clone()).map_err(bad)?,
            since: uint("since")?,
            gone: uint("gone")?,
            signature: (!signature.is_empty()).then_some(Signature {
                key: signer,
                sig: signature,
            }),
        })
    }
    fn document(&self) -> Value {
        json!({"id":value::encoded(&self.id),"namespace":self.namespace,"digest":self.digest,"intent":self.intent,"state":self.state,
            "labels":self.labels,"since":self.since,"gone":self.gone,"signature":self.signature.as_ref().map(|s|s.sig.as_str()).unwrap_or(""),
            "signedBy":self.signature.as_ref().map(|s|s.key.as_str()).unwrap_or("")})
    }
}
impl Transaction<'_> {
    pub(crate) fn manifest_entity(&self) -> Result<(&Entity, u8)> {
        if !matches!(self.scope, Scope::Internal) {
            return Err(Fault::denied("manifest updates require a host transaction"));
        }
        let (name, policy) = self
            .policies
            .iter()
            .find(|(_, p)| p.entity["own"] == "OWN_ARCHIVED")
            .ok_or_else(|| bad("missing Archived schema"))?;
        let domain = policy.entity["domain"]
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .ok_or_else(|| bad("missing manifest domain"))?;
        Ok((self.model.entity(name).map_err(bad)?, domain))
    }
    pub fn archive_intent(
        &mut self,
        namespace: &str,
        intent: &str,
        labels: BTreeMap<String, String>,
        sign: impl FnOnce(Option<&Record>, &Record) -> Result<Option<Signature>>,
    ) -> Result<Record> {
        if namespace.is_empty() || intent.is_empty() {
            return Err(Fault::invalid(
                "manifest intent requires namespace and intent",
            ));
        }
        let (entity, domain) = self.manifest_entity()?;
        let entity = entity.clone();
        let mut r = Record {
            id: (self.inputs.mint)(domain),
            namespace: namespace.into(),
            intent: intent.into(),
            digest: String::new(),
            state: State::Adding,
            labels,
            since: 0,
            gone: 0,
            signature: None,
        };
        r.signature = sign(None, &r)?;
        self.insert(&entity, &r.document())?;
        Ok(r)
    }
    pub fn archive_record(&mut self, id: [u8; 16]) -> Result<Record> {
        let (entity, _) = self.manifest_entity()?;
        let entity = entity.clone();
        Record::decode(&self.lookup(&entity, &json!({"id":value::encoded(&id)}), false)?)
    }
    /// Applies a signed mutation only while the entire observed row is still current.
    pub fn change_archive_record(
        &mut self,
        id: [u8; 16],
        expected: State,
        change: impl FnOnce(&mut Record) -> Result<()>,
        sign: impl FnOnce(Option<&Record>, &Record) -> Result<Option<Signature>>,
    ) -> Result<Option<Record>> {
        let (entity, _) = self.manifest_entity()?;
        let entity = entity.clone();
        let raw = self.lookup(&entity, &json!({"id":value::encoded(&id)}), false)?;
        let was = Record::decode(&raw)?;
        if was.state != expected {
            return Ok(None);
        }
        let mut next = was.clone();
        change(&mut next)?;
        if next.id != id || next.namespace.is_empty() || next.intent.is_empty() {
            return Err(Fault::invalid("invalid manifest transition"));
        }
        next.signature = sign(Some(&was), &next)?;
        let mut doc = next.document();
        doc["dateUpdated"] = value::next_stamp(&raw["dateUpdated"], (self.inputs.clock)())?;
        let d = self.db.dialect();
        let mut q = Query::new("");
        let mut set = Vec::new();
        let mut condition = Vec::new();
        for f in &entity.fields {
            if !f.is_column() {
                continue;
            }
            let col = quote(&f.column());
            if !f.immutable && f.name != entity.key && doc.get(&f.json).is_some() {
                let b = q.param(value::bind(f, &doc[&f.json])?, d);
                set.push(format!("{col}={b}"));
            }
            if !raw[&f.json].is_null() {
                let b = q.param(value::bind(f, &raw[&f.json])?, d);
                condition.push(format!("{col}={b}"));
            } else {
                condition.push(format!("{col} IS NULL"));
            }
        }
        q.sql = format!(
            "UPDATE {} SET {} WHERE {}",
            quote(&entity.table()),
            set.join(","),
            condition.join(" AND ")
        );
        Ok((self.db.exec(q)? == 1).then_some(next))
    }
    pub(crate) fn manifest_rows(&mut self) -> Result<Vec<Record>> {
        let (e, _) = self.manifest_entity()?;
        let e = e.clone();
        let mut q = Query::new("");
        let zero = q.param(Bind::Bytes(vec![0; 16]), self.db.dialect());
        q.sql = format!(
            "SELECT {} AS value FROM {} r WHERE r.{}<>{zero} ORDER BY r.{}",
            self.project_sql(&e)?,
            quote(&e.table()),
            quote(&e.key().map_err(bad)?.column()),
            quote(&e.key().map_err(bad)?.column())
        );
        self.db
            .rows(q)?
            .into_iter()
            .map(|raw| Record::decode(&self.decode_row(&e, &raw)?))
            .collect()
    }
}

impl Transaction<'_> {
    pub(crate) fn checkpoint_counter(&mut self) -> Result<Option<u64>> {
        let (e, _) = self.manifest_entity()?;
        let e = e.clone();
        let mut q = Query::new("");
        let zero = q.param(Bind::Bytes(vec![0; 16]), self.db.dialect());
        q.sql = format!(
            "SELECT CAST({} AS TEXT) AS value FROM {} WHERE {}={zero}",
            quote(&e.field("since").map_err(bad)?.column()),
            quote(&e.table()),
            quote(&e.key().map_err(bad)?.column())
        );
        self.db
            .rows(q)?
            .first()
            .map(|v| v.parse().map_err(bad))
            .transpose()
    }
    pub(crate) fn set_checkpoint_counter(&mut self, k: u64) -> Result<()> {
        let (e, _) = self.manifest_entity()?;
        let e = e.clone();
        let mut q = Query::new("");
        let d = self.db.dialect();
        let mut cols = Vec::new();
        let mut params = Vec::new();
        let number = u32::try_from(k).map_err(|_| bad("manifest checkpoint counter overflow"))?;
        for field in e.fields.iter().filter(|f| f.is_column() && !f.nullable) {
            let val = match field.name.as_str() {
                "id" => value::encoded(&[0; 16]),
                "since" => json!(number),
                "labels" => json!({}),
                _ => value::default_value(field, self.inputs),
            };
            cols.push(quote(&field.column()));
            params.push(q.param(value::bind(field, &val)?, d));
        }
        let since = quote(&e.field("since").map_err(bad)?.column());
        let version = quote(&e.field("date_updated").map_err(bad)?.column());
        let table = quote(&e.table());
        let key = quote(&e.key().map_err(bad)?.column());
        q.sql = format!(
            "INSERT INTO {table} ({}) VALUES ({}) ON CONFLICT ({key}) DO UPDATE SET {since}=excluded.{since},{version}=excluded.{version}",
            cols.join(","),
            params.join(",")
        );
        self.db.exec(q)?;
        let state = quote(&e.field("state").map_err(bad)?.column());
        let gone = quote(&e.field("gone").map_err(bad)?.column());
        for (col, status) in [(since, "present"), (gone, "erased")] {
            let mut q = Query::new("");
            let num = q.param(Bind::Big(i64::from(number)), d);
            q.sql = format!("UPDATE {table} SET {col}={num} WHERE {state}='{status}' AND {col}=0");
            self.db.exec(q)?;
        }
        Ok(())
    }
}
