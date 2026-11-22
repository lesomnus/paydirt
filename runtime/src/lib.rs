//! Transactional entity storage for generated Payday applications.
mod crud;
mod driver;
mod value;
pub use driver::Connection;
use driver::{Bind, Driver, Query};
use paydirt_model::{Dialect, Model};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub type Result<T> = std::result::Result<T, Fault>;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fault {
    pub code: String,
    pub message: String,
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Fault {}
macro_rules! faults {($($f:ident=>$code:literal),*)=>{$(pub fn $f(msg:impl Into<String>)->Self{Self{code:$code.into(),message:msg.into()}})*};}
impl Fault {
    faults!(invalid=>"invalid_argument", conflict=>"already_exists",precondition=>"failed_precondition",aborted=>"aborted",internal=>"internal",unsupported=>"unimplemented",denied=>"permission_denied",unauthenticated=>"unauthenticated");
    pub fn missing() -> Self {
        Self {
            code: "not_found".into(),
            message: "row not found".into(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Scope {
    Internal,
    Tenant(Vec<u8>),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Call {
    pub entity: String,
    pub method: String,
    pub request: Value,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Policy {
    #[serde(default)]
    pub entity: Value,
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
    #[serde(default)]
    pub methods: BTreeMap<String, Value>,
}
pub type Policies = BTreeMap<String, Policy>;
pub struct Store {
    db: Connection,
    model: Model,
    policies: Policies,
}
impl Store {
    pub fn open(url: &str, model_json: &str, policy_json: &str) -> Result<Self> {
        let model = Model::parse(model_json.as_bytes()).map_err(Fault::invalid)?;
        let policies =
            serde_json::from_str(policy_json).map_err(|e| Fault::invalid(e.to_string()))?;
        let mut db = Connection::open(url)?;
        db.transaction(|tx| install(tx, &model))?;
        Ok(Self {
            db,
            model,
            policies,
        })
    }
    pub fn call(&mut self, scope: &Scope, call: Call) -> Result<Value> {
        self.transaction(scope, |tx| tx.call(call))
    }
    pub fn transaction<T>(
        &mut self,
        scope: &Scope,
        f: impl FnOnce(&mut Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let (model, policies) = (&self.model, &self.policies);
        self.db.transaction(|db| {
            let mut tx = Transaction {
                db,
                model,
                policies,
                scope,
                failed: None,
            };
            let result = f(&mut tx);
            match tx.failed {
                Some(err) => Err(err),
                None => result,
            }
        })
    }
}
pub struct Transaction<'a> {
    db: &'a mut dyn Driver,
    model: &'a Model,
    policies: &'a Policies,
    scope: &'a Scope,
    failed: Option<Fault>,
}
impl Transaction<'_> {
    pub fn call(&mut self, call: Call) -> Result<Value> {
        if let Some(failure) = &self.failed {
            return Err(failure.clone());
        }
        let result = self
            .model
            .entity(&call.entity)
            .map_err(Fault::invalid)
            .and_then(|entity| self.dispatch(entity, &call.method, &call.request));
        if let Err(err) = &result {
            self.failed = Some(err.clone());
        }
        result
    }
}
fn install(db: &mut dyn Driver, model: &Model) -> Result<()> {
    // Serialize two processes creating or upgrading the same application database.
    if db.dialect() == Dialect::Postgres {
        db.batch("SELECT pg_advisory_xact_lock(7174429307262620)")?
    }
    db.batch(
        "CREATE TABLE IF NOT EXISTS paydirt_schema (id INTEGER PRIMARY KEY, model TEXT NOT NULL)",
    )?;
    let existing = db.rows(Query::new(
        "SELECT model AS value FROM paydirt_schema WHERE id=1",
    ))?;
    if let Some(old) = existing.first() {
        let old = Model::parse(old.as_bytes()).map_err(Fault::precondition)?;
        if old != *model {
            return Err(Fault::precondition(
                "stored ORM schema differs from the application schema",
            ));
        }
        return Ok(());
    }
    db.batch(&model.ddl(db.dialect()).map_err(Fault::invalid)?)?;
    let mut q = Query::new("");
    let p = q.param(
        Bind::Text(serde_json::to_string(model).map_err(|e| Fault::internal(e.to_string()))?),
        db.dialect(),
    );
    q.sql = format!("INSERT INTO paydirt_schema (id,model) VALUES (1,{p})");
    db.exec(q)?;
    Ok(())
}
