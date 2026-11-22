use crate::{Call, Fault, Result, Scope, Store, Transaction};
use paydirt_model::Dialect;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A protobuf JSON row resolved for an authenticated principal by the host.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrincipalRow {
    pub entity: String,
    pub value: Value,
}

/// Established by the host's credential resolver before calling the application.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub tenant: Vec<u8>,
    #[serde(default)]
    pub account: Option<Vec<u8>>,
    #[serde(default)]
    pub topics: crate::TopicGrants,
    #[serde(default)]
    pub actor: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<PrincipalRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_row: Option<PrincipalRow>,
    #[serde(default)]
    pub tenants: Option<Vec<Vec<u8>>>,
    #[serde(default)]
    pub sets: Option<Vec<Vec<u8>>>,
    #[serde(default)]
    pub caller: Option<Vec<u8>>,
    #[serde(default)]
    pub caller_tenant: Option<Vec<u8>>,
    #[serde(default)]
    pub presence: Option<String>,
    #[serde(default)]
    pub trace: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterpart: Option<Vec<u8>>,
    pub action: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Packet {
    Observe(crate::access::Observation),
    BufferObservation(crate::access::Observation),
    FlushAccess,
    BeginOrigin {
        id: String,
        frame: Frame,
        origin: Call,
    },
    ResolveAccount {
        account: Vec<u8>,
        bound: Option<Vec<u8>>,
        selector: String,
    },
    Begin {
        id: String,
        frame: Frame,
    },
    AccountCall {
        id: String,
        call: Call,
        reads: crate::AccountReads,
    },
    Call {
        id: String,
        call: Call,
    },
    Concerning {
        id: String,
        tenant: Option<Vec<u8>>,
    },
    Settle {
        id: String,
        commit: bool,
    },
    ObservedNames {
        observation: crate::access::Observation,
        named: crate::access::Subjects,
        refused: crate::access::Subjects,
        buffered: bool,
    },
    RefusedWrites {
        observation: crate::access::Observation,
        subjects: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
        buffered: bool,
    },
    Finish {
        id: String,
        commit: bool,
    },
}
pub(crate) struct Session {
    written: crate::access::Subjects,
    operations: std::collections::BTreeSet<String>,
    origin: Option<Call>,
    pub seen: crate::seen::Seen,
    changes: Vec<paydirt::watch::Change>,
    pub id: String,
    pub frame: Frame,
    pub failed: Option<Fault>,
}

impl Store {
    /// The server serializes transactions, including across its private Worker port.
    /// Only a session opened on this connection can issue operations or finish it.
    pub fn handle(&mut self, packet: Packet) -> Result<Value> {
        match packet {
            Packet::Concerning { id, tenant } => {
                if tenant.as_ref().is_some_and(|v| !paydirt::has_domain(v, 1)) {
                    return Err(Fault::invalid("expected a tenant identifier"));
                }
                let session = self
                    .session
                    .as_mut()
                    .filter(|s| s.id == id)
                    .ok_or_else(|| Fault::denied("unknown request transaction"))?;
                session.frame.counterpart = tenant;
                Ok(Value::Null)
            }
            Packet::Settle { id, commit } => {
                let groups = self
                    .session
                    .as_ref()
                    .filter(|s| s.id == id)
                    .ok_or_else(|| Fault::denied("unknown request transaction"))?
                    .written
                    .clone();
                let result = self.handle(Packet::Finish { id, commit });
                let refused = if !commit || result.is_err() {
                    groups
                } else {
                    Default::default()
                };
                Ok(match result {
                    Ok(receipt) => serde_json::json!({"receipt":receipt,"refused":refused}),
                    Err(fault) => serde_json::json!({"fault":fault,"refused":refused}),
                })
            }
            Packet::ObservedNames {
                observation,
                named,
                refused,
                buffered,
            } => self.observe_named(observation, buffered, refused, named),
            Packet::RefusedWrites {
                observation,
                subjects,
                buffered,
            } => self.observe_with_writes(observation, buffered, subjects),
            Packet::Observe(o) => self.observe(o, false),
            Packet::BufferObservation(o) => self.observe(o, true),
            Packet::FlushAccess => self.flush_access().map(|n| serde_json::json!(n)),
            Packet::ResolveAccount {
                account,
                bound,
                selector,
            } => self.transaction(&Scope::Internal, |tx| {
                tx.resolve_account(&account, bound.as_deref(), &selector)
            }),
            Packet::BeginOrigin { id, frame, origin } => {
                let result = self.handle(Packet::Begin {
                    id: id.clone(),
                    frame,
                })?;
                if matches!(origin.method.as_str(), "Add" | "AddMany")
                    && self.model.entity(&origin.entity).is_ok()
                {
                    let preflight = Call {
                        entity: origin.entity.clone(),
                        method: if origin.method == "Add" {
                            "$check_add"
                        } else {
                            "$check_add_many"
                        }
                        .into(),
                        request: origin.request.clone(),
                    };
                    if let Err(err) = self.session_call(id.clone(), preflight, None) {
                        self.handle(Packet::Finish { id, commit: false })?;
                        return Err(err);
                    }
                }
                self.session.as_mut().unwrap().origin = Some(origin);
                Ok(result)
            }
            Packet::Begin { id, frame } => {
                if self.session.is_some() {
                    return Err(Fault::aborted("another request owns this connection"));
                }
                let as_account = frame.tenant.is_empty()
                    && frame.account.as_ref().is_some_and(|a| {
                        a.len() == 16
                            && a.iter().any(|&b| b != 0)
                            && frame.actor.as_ref() == Some(a)
                    });
                if (!paydirt::has_domain(&frame.tenant, 1) && !as_account) || id.is_empty() {
                    return Err(Fault::unauthenticated("invalid request frame"));
                }
                let db = self.db.driver()?;
                db.batch(if db.dialect() == Dialect::Sqlite {
                    "BEGIN IMMEDIATE"
                } else {
                    "BEGIN"
                })?;
                self.session = Some(Session {
                    origin: None,
                    written: Default::default(),
                    operations: Default::default(),
                    seen: Default::default(),
                    changes: Vec::new(),
                    id,
                    frame,
                    failed: None,
                });
                Ok(Value::Null)
            }
            Packet::Call { id, call } => self.session_call(id, call, None),
            Packet::AccountCall { id, call, reads } => self.session_call(id, call, Some(reads)),
            Packet::Finish { id, commit } => {
                if !self.session.as_ref().is_some_and(|s| s.id == id) {
                    return Err(Fault::denied("unknown request transaction"));
                }
                let mut session = self.session.take().expect("validated session");
                let db = self.db.driver()?;
                let failed = session.failed.take();
                let written: Vec<_> = session.written.values().flatten().cloned().collect();
                if commit && failed.is_none() {
                    let mut tx = Transaction {
                        scope_rules: self.scope_rules.as_deref().unwrap_or_default(),
                        pending_access: None,
                        admission: None,
                        origin: None,
                        seen: None,
                        changes: &mut session.changes,
                        outbox: false,
                        trail: self.trail.as_ref(),
                        resources: None,
                        topic_policy: None,
                        inputs: &self.inputs,
                        namer: &*self.namer,
                        db,
                        model: &self.model,
                        policies: &self.policies,
                        scope: &Scope::Internal,
                        frame: None,
                        failed: None,
                    };
                    let result = tx
                        .flush_written(&session.frame, std::mem::take(&mut session.written))
                        .and_then(|_| tx.db.batch("COMMIT"));
                    if let Err(err) = result {
                        tx.db.batch("ROLLBACK")?;
                        return Err(err);
                    }
                } else {
                    db.batch("ROLLBACK")?;
                }
                if !commit {
                    return Ok(Value::Null);
                }
                if let Some(err) = failed {
                    return Err(err);
                }
                if session.changes.is_empty() {
                    return Ok(Value::Null);
                }
                let event = paydirt::watch::Event {
                    actor: session
                        .frame
                        .actor
                        .as_deref()
                        .and_then(paydirt::format_id)
                        .unwrap_or_default(),
                    tenant: paydirt::format_id(&session.frame.tenant).unwrap_or_default(),
                    method: session.frame.action,
                    changes: session.changes,
                };
                let mut receipt =
                    serde_json::to_value(event).map_err(|e| Fault::internal(e.to_string()))?;
                receipt["writtenSubjects"] = serde_json::json!(written);
                Ok(receipt)
            }
        }
    }
    fn session_call(
        &mut self,
        id: String,
        call: Call,
        reads: Option<crate::AccountReads>,
    ) -> Result<Value> {
        let session = self
            .session
            .as_mut()
            .filter(|s| s.id == id)
            .ok_or_else(|| Fault::denied("unknown request transaction"))?;
        if let Some(e) = &session.failed {
            return Err(e.clone());
        }
        let call_method_children = call.method == "$event_children";
        let beginning = call.method == "$begin_event";
        let reference = match call.method.as_str() {
            "$begin_event" | "$event" => call.request["cause"].as_str(),
            "$event_facts" | "$event_children" | "$correct_fact" | "$correct_event" => {
                Some(call.request["event"].as_str().unwrap_or(""))
            }

            _ => None,
        };
        if reference.is_some_and(|key| !session.operations.contains(key)) {
            return Err(Fault::precondition(
                "operation belongs to another transaction",
            ));
        }
        let scope = if session.frame.tenant.is_empty() {
            Scope::Account {
                id: session.frame.account.clone().unwrap(),
                reads: reads.unwrap_or_default(),
            }
        } else {
            Scope::Granted {
                tenants: session
                    .frame
                    .tenants
                    .clone()
                    .unwrap_or_else(|| vec![session.frame.tenant.clone()]),
                sets: session.frame.sets.clone(),
            }
        };
        let mut tx = Transaction {
            scope_rules: self.scope_rules.as_deref().unwrap_or_default(),
            pending_access: Some(&mut session.written),
            admission: self.admission.as_ref(),
            origin: session.origin.clone(),
            seen: (!session.frame.tenant.is_empty()).then_some(&mut session.seen),
            changes: &mut session.changes,
            outbox: self.outbox,
            trail: self.trail.as_ref(),
            resources: self.resources.as_ref(),
            topic_policy: self.topic_policy.as_deref(),
            inputs: &self.inputs,
            namer: &*self.namer,
            db: self.db.driver()?,
            policies: &self.policies,
            model: &self.model,
            scope: &scope,
            frame: Some(&session.frame),
            failed: None,
        };
        let result = if matches!(call.method.as_str(), "$check_add" | "$check_add_many")
            && !session.frame.tenant.is_empty()
        {
            let e = tx.model.entity(&call.entity).map_err(Fault::invalid)?;
            tx.check_add_input(e, &call.request, call.method == "$check_add_many")
        } else if session.frame.tenant.is_empty() {
            let declared = self.policies.get(&call.entity).is_some_and(|p| {
                p.entity["own"] == "OWN_HOLDER"
                    && p.methods.get("Memberships").unwrap_or(&Value::Null)["policy"]
                        .get("account")
                        .is_some()
            });
            let selected = session
                .frame
                .action
                .strip_prefix('/')
                .and_then(|s| s.rsplit_once("Service/"))
                .and_then(|(e, m)| self.policies.get(e)?.methods.get(m))
                .is_some_and(|m| m["policy"].get("account").is_some());
            let direct = self
                .policies
                .get(&call.entity)
                .and_then(|p| p.methods.get(&call.method))
                .is_some_and(|m| m["policy"].get("account").is_some());
            if !selected && !direct {
                Err(Fault::denied(
                    "account credential requires an account method",
                ))
            } else if self
                .policies
                .get(&call.entity)
                .is_some_and(|p| p.notification)
                && call.entity.ends_with(".NotificationAccount")
                && session
                    .frame
                    .action
                    .starts_with(&format!("/{}Service/", call.entity))
            {
                tx.notify(&call.entity, &call.method, &call.request)
            } else if declared
                && (session.frame.action.is_empty()
                    || session.frame.action == format!("/{}Service/Memberships", call.entity))
                && call.method == "Memberships"
            {
                let mut reader = Transaction {
                    scope_rules: tx.scope_rules,
                    pending_access: tx.pending_access.as_deref_mut(),
                    admission: None,
                    origin: None,
                    seen: None,
                    changes: tx.changes,
                    outbox: tx.outbox,
                    trail: tx.trail,
                    resources: tx.resources,
                    topic_policy: tx.topic_policy,
                    inputs: tx.inputs,
                    namer: tx.namer,
                    db: tx.db,
                    model: tx.model,
                    policies: tx.policies,
                    scope: &Scope::Internal,
                    frame: tx.frame,
                    failed: None,
                };
                reader
                    .memberships(session.frame.account.as_ref().unwrap(), None, "", false)
                    .map(|rows| serde_json::json!({"items":rows}))
            } else if matches!(call.method.as_str(), "Get" | "List") {
                tx.call(call)
            } else {
                Err(Fault::denied(
                    "account method has no declared read of this entity",
                ))
            }
        } else {
            tx.call(call)
        };
        if beginning
            && let Ok(row) = &result
            && row["done"] != true
            && let Some(id) = row["id"].as_str()
        {
            session.operations.insert(id.into());
        }
        if call_method_children && let Ok(children) = &result {
            for child in children
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| r["done"] != true)
            {
                if let Some(id) = child["id"].as_str() {
                    session.operations.insert(id.to_owned());
                }
            }
        }
        session.failed = result.as_ref().err().cloned().or(tx.failed);
        result
    }
}
