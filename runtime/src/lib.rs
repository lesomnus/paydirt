//! Transactional entity storage for generated Payday applications.
mod account;
mod apply;
pub mod auth;
mod crud;
mod driver;
pub mod entitle;
mod list;
pub mod meter;
mod migrate;
pub mod naming;
mod notify;
mod past;
mod relations;
mod session;
mod tenancy;
mod trail;
mod value;
mod watch;
pub use driver::Connection;
use driver::Driver;
pub use notify::TopicGrants;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Fault {}
macro_rules! faults {($($f:ident=>$code:literal),*)=>{$(pub fn $f(msg:impl Into<String>)->Self{Self{code:$code.into(),message:msg.into(),field:None}})*};}
impl Fault {
    faults!(invalid=>"invalid_argument", conflict=>"already_exists",precondition=>"failed_precondition",aborted=>"aborted",internal=>"internal",unsupported=>"unimplemented",denied=>"permission_denied",unauthenticated=>"unauthenticated");
    pub fn field(name: &str, message: &str) -> Self {
        Self {
            code: "invalid_argument".into(),
            message: message.into(),
            field: Some(name.into()),
        }
    }
    pub fn missing() -> Self {
        Self {
            code: "not_found".into(),
            message: "row not found".into(),
            field: None,
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
    pub notification: bool,
    #[serde(default)]
    pub application: Value,
    #[serde(default)]
    pub entity: Value,
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
    #[serde(default)]
    pub methods: BTreeMap<String, Value>,
}
pub type Policies = BTreeMap<String, Policy>;
pub struct Store {
    namer: Box<dyn naming::Namer>,
    db: Connection,
    session: Option<session::Session>,
    model: Model,
    policies: Policies,
}
impl Store {
    pub fn open(url: &str, model_json: &str, policy_json: &str) -> Result<Self> {
        let model = Model::parse(model_json.as_bytes()).map_err(Fault::invalid)?;
        let policies: Policies =
            serde_json::from_str(policy_json).map_err(|e| Fault::invalid(e.to_string()))?;
        let mut db = Connection::open(url)?;
        db.transaction(|tx| {
            migrate::install(tx, &model)?;
            if policies.values().any(|p| p.notification) {
                notify::install(tx)?;
            }
            watch::install(tx)
        })?;
        Ok(Self {
            namer: Box::new(naming::names),
            db,
            session: None,
            model,
            policies,
        })
    }
    pub fn with_namer(mut self, namer: impl naming::Namer + 'static) -> Self {
        self.namer = Box::new(namer);
        self
    }

    /// Host-only operations used by the application executable and sandbox bootstrap.
    pub fn operator(&mut self, call: Call) -> Result<Value> {
        let p = self.policies.get(&call.entity);
        if p.is_some_and(|p| p.entity["own"] == "OWN_TENANT") && call.method == "Measure" {
            let tenant = value::bytes(&call.request["id"])?;
            return serde_json::to_value(self.measure(&tenant)?)
                .map_err(|e| Fault::internal(e.to_string()));
        }
        if p.is_some_and(|p| p.entity["own"] == "OWN_ENTITLEMENT") {
            match call.method.as_str() {
                "Follow" => {
                    let events = call.request["events"]
                        .as_array()
                        .ok_or_else(|| Fault::invalid("grant events required"))?;
                    let mut pages = call.request["pages"].as_array().into_iter().flatten();
                    self.follow_grants(events.iter().cloned().map(Ok), |_| {
                        pages
                            .next()
                            .cloned()
                            .ok_or_else(|| Fault::precondition("incomplete grant list"))
                    })?;
                    return Ok(Value::Null);
                }
                "Sync" => {
                    let tenant = call.request.get("tenantId").map(value::bytes).transpose()?;
                    let mut pages = call.request["pages"].as_array().into_iter().flatten();
                    let report = self.reconcile_grants(tenant.as_deref(), |_| {
                        pages
                            .next()
                            .cloned()
                            .ok_or_else(|| Fault::precondition("incomplete grant list"))
                    })?;
                    return serde_json::to_value(report)
                        .map_err(|e| Fault::internal(e.to_string()));
                }
                _ => {}
            }
        }
        self.call(&Scope::Internal, call)
    }
    pub fn seed(&mut self, rows: Vec<(String, Value)>) -> Result<()> {
        self.transaction(&Scope::Internal, |tx| {
            for (name, row) in rows {
                let e = tx.model.entity(&name).map_err(Fault::invalid)?;
                let key = &e.key().map_err(Fault::invalid)?.json;
                let id = row
                    .get(key)
                    .ok_or_else(|| Fault::invalid("seed requires a stable key"))?;
                match tx.lookup(e, &serde_json::json!({key:id}), true) {
                    Ok(_) => {}
                    Err(f) if f.code == "not_found" => {
                        tx.insert(e, &row)?;
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok(())
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
                namer: &*self.namer,
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
    namer: &'a dyn naming::Namer,
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
        let result = if self
            .policies
            .get(&call.entity)
            .is_some_and(|p| p.notification)
        {
            self.notify(&call.entity, &call.method, &call.request)
        } else {
            self.model
                .entity(&call.entity)
                .map_err(Fault::invalid)
                .and_then(|entity| self.dispatch(entity, &call.method, &call.request))
        };
        if let Err(err) = &result {
            self.failed = Some(err.clone());
        }
        result
    }
}

#[cfg(all(test, any(feature = "sqlite", feature = "postgres")))]
mod session_test;
