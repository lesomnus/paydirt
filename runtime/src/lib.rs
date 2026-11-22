//! Transactional entity storage for generated Payday applications.
mod account;
pub use account::{AccountReads, ReadFilter};
pub mod access;
pub mod admission;
mod apply;
#[cfg(not(target_arch = "wasm32"))]
pub mod archive_files;
#[cfg(not(target_arch = "wasm32"))]
pub mod attestation;
pub mod audit_content;
pub mod auth;
mod crud;
pub mod domain;
pub mod header;
pub mod integrity;
#[cfg(not(target_arch = "wasm32"))]
pub mod rekor;
#[cfg(not(target_arch = "wasm32"))]
pub mod retention;
pub use paydirt_model;
mod driver;
pub mod entitle;
mod list;
pub mod meter;
mod migrate;
pub mod naming;
pub mod notify;
pub mod outbox;
mod past;
pub mod past_expiry;
mod relations;
pub mod resource;
pub mod secrets;
mod seen;
mod session;
mod tenancy;
mod trail;
pub mod trail_page;
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
pub struct Violation {
    pub field: String,
    pub description: String,
}
impl Violation {
    pub fn new(field: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            description: description.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fault {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub violations: Vec<Violation>,
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Fault {}
macro_rules! faults {($($f:ident=>$code:literal),*)=>{$(pub fn $f(msg:impl Into<String>)->Self{Self{code:$code.into(),message:msg.into(),field:None,violations:Vec::new()}})*};}
impl Fault {
    faults!(unavailable=>"unavailable",deadline=>"deadline_exceeded",exhausted=>"resource_exhausted",invalid=>"invalid_argument", conflict=>"already_exists",precondition=>"failed_precondition",aborted=>"aborted",internal=>"internal",unsupported=>"unimplemented",denied=>"permission_denied",unauthenticated=>"unauthenticated");
    pub fn field(name: &str, message: &str) -> Self {
        Self {
            code: "invalid_argument".into(),
            message: message.into(),
            field: Some(name.into()),
            violations: Vec::new(),
        }
    }
    pub fn validation(violations: impl IntoIterator<Item = Violation>) -> Self {
        let violations: Vec<_> = violations.into_iter().collect();
        if violations.is_empty() {
            return Self::internal("a request was refused and nothing said why");
        }
        let message = violations
            .iter()
            .map(|v| {
                if v.field.is_empty() {
                    v.description.clone()
                } else {
                    format!("{}: {}", v.field, v.description)
                }
            })
            .collect::<Vec<_>>()
            .join("; ");
        Self {
            code: "invalid_argument".into(),
            message,
            field: None,
            violations,
        }
    }
    pub fn at(self, parent: &str) -> Self {
        if self.code != "invalid_argument" && self.code != "unknown" {
            return self;
        }
        let mut violations = self.violations;
        if violations.is_empty() {
            violations.push(Violation::new(self.field.unwrap_or_default(), self.message))
        }
        for v in &mut violations {
            if !parent.is_empty() {
                v.field = if v.field.is_empty() {
                    parent.to_owned()
                } else if v.field.starts_with('[') {
                    format!("{parent}{}", v.field)
                } else {
                    format!("{parent}.{}", v.field)
                };
            }
        }
        Self::validation(violations)
    }
    pub fn missing() -> Self {
        Self {
            code: "not_found".into(),
            message: "row not found".into(),
            field: None,
            violations: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Scope {
    Internal,
    Tenant(Vec<u8>),
    Account {
        id: Vec<u8>,
        reads: AccountReads,
    },
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
/// Time and identity sources shared by all writes in a store, including nested writes.
pub struct Inputs {
    clock: Box<dyn Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync>,
    mint: Box<dyn Fn(u8) -> [u8; 16] + Send + Sync>,
}
impl Inputs {
    pub fn new(
        clock: impl Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync + 'static,
        mint: impl Fn(u8) -> [u8; 16] + Send + Sync + 'static,
    ) -> Self {
        Self {
            clock: Box::new(clock),
            mint: Box::new(mint),
        }
    }
    fn stamp(&self) -> Value {
        serde_json::json!((self.clock)().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
    }
}
impl Default for Inputs {
    fn default() -> Self {
        Self::new(chrono::Utc::now, paydirt::mint)
    }
}
pub struct Store {
    trail: Option<audit_content::Recorder>,
    resources: Option<resource::Recorder>,
    topic_policy: Option<Box<dyn notify::Policy>>,
    inputs: Inputs,
    admission: Option<admission::Rules>,
    outbox: bool,
    committed: Option<paydirt::watch::Event>,
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
            migrate::install(tx, &model, &policies)?;
            if policies.values().any(|p| p.notification) {
                notify::install(tx)?;
            }
            watch::install(tx)
        })?;
        Ok(Self {
            trail: None,
            resources: None,
            topic_policy: None,
            admission: None,
            inputs: Inputs::default(),
            namer: Box::new(naming::names),
            outbox: true,
            committed: None,
            db,
            session: None,
            model,
            policies,
        })
    }
    pub fn with_inputs(mut self, inputs: Inputs) -> Self {
        self.inputs = inputs;
        self
    }
    pub fn with_outbox(mut self, enabled: bool) -> Self {
        self.outbox = enabled;
        self
    }
    pub fn take_committed(&mut self) -> Option<paydirt::watch::Event> {
        self.committed.take()
    }
    pub fn has_admission(&self) -> bool {
        self.admission.is_some()
    }
    pub fn with_admission(
        mut self,
        schema: &str,
        options: admission::Options,
        policy: impl Fn(admission::Decision) -> Result<()> + Send + Sync + 'static,
    ) -> Result<Self> {
        self.admission = Some(admission::Rules::new(
            schema,
            &self.policies,
            options,
            std::sync::Arc::new(policy),
        )?);
        Ok(self)
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
        let mut changes = Vec::new();
        self.committed = None;
        let result = self.db.transaction(|db| {
            let mut seen = seen::Seen::default();
            let mut tx = Transaction {
                admission: self.admission.as_ref(),
                origin: None,
                seen: Some(&mut seen),
                changes: &mut changes,
                outbox: self.outbox,
                trail: self.trail.as_ref(),
                resources: self.resources.as_ref(),
                topic_policy: self.topic_policy.as_deref(),
                inputs: &self.inputs,
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
        })?;
        if !changes.is_empty() {
            self.committed = Some(paydirt::watch::Event {
                changes,
                ..Default::default()
            });
        }
        Ok(result)
    }
}
pub struct Transaction<'a> {
    trail: Option<&'a audit_content::Recorder>,
    resources: Option<&'a resource::Recorder>,
    topic_policy: Option<&'a dyn notify::Policy>,
    admission: Option<&'a admission::Rules>,
    origin: Option<Call>,
    seen: Option<&'a mut seen::Seen>,
    changes: &'a mut Vec<paydirt::watch::Change>,
    outbox: bool,
    inputs: &'a Inputs,
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
        if matches!(self.scope, Scope::Account { .. })
            && !matches!(call.method.as_str(), "Get" | "List")
        {
            return Err(Fault::denied("account read scopes permit Get and List"));
        }
        if let Some(failure) = &self.failed {
            return Err(failure.clone());
        }
        let implicit = self.admission.is_some() && self.origin.is_none();
        if implicit {
            self.origin = Some(call.clone());
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
        if implicit {
            self.origin = None;
        }
        if let Err(err) = &result {
            self.failed = Some(err.clone());
        }
        result
    }
}

#[cfg(all(test, any(feature = "sqlite", feature = "postgres")))]
mod session_test;

#[cfg(all(feature = "resource-cel", not(target_arch = "wasm32")))]
pub mod resource_filter;
