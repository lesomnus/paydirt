//! Transactional entity storage for generated Payday applications.
mod crud;
mod driver;
mod migrate;
mod session;
mod tenancy;
mod trail;
mod value;
pub use driver::Connection;
use driver::Driver;
use paydirt_model::Model;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use session::{Frame, Packet};
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
    Granted {
        tenants: Vec<Vec<u8>>,
        sets: Option<Vec<Vec<u8>>>,
    },
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
    session: Option<session::Session>,
    model: Model,
    policies: Policies,
}
impl Store {
    pub fn open(url: &str, model_json: &str, policy_json: &str) -> Result<Self> {
        let model = Model::parse(model_json.as_bytes()).map_err(Fault::invalid)?;
        let policies =
            serde_json::from_str(policy_json).map_err(|e| Fault::invalid(e.to_string()))?;
        let mut db = Connection::open(url)?;
        db.transaction(|tx| migrate::install(tx, &model))?;
        Ok(Self {
            db,
            session: None,
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
        if self.session.is_some() {
            return Err(Fault::aborted("another request owns this connection"));
        }
        let (model, policies) = (&self.model, &self.policies);
        self.db.transaction(|db| {
            let mut tx = Transaction {
                db,
                model,
                policies,
                scope,
                frame: None,
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
    frame: Option<&'a Frame>,
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

#[cfg(all(test, any(feature = "sqlite", feature = "postgres")))]
mod session_test;
