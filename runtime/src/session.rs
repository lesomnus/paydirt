use crate::{Call, Fault, Result, Scope, Store, Transaction};
use paydirt_model::Dialect;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    #[serde(default)]
    pub tenants: Option<Vec<Vec<u8>>>,
    #[serde(default)]
    pub sets: Option<Vec<Vec<u8>>>,
    #[serde(default)]
    pub caller: Option<Vec<u8>>,
    #[serde(default)]
    pub caller_tenant: Option<Vec<u8>>,
    #[serde(default)]
    pub trace: Vec<u8>,
    pub action: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Packet {
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
    Call {
        id: String,
        call: Call,
    },
    Finish {
        id: String,
        commit: bool,
    },
}
pub(crate) struct Session {
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
            Packet::ResolveAccount {
                account,
                bound,
                selector,
            } => self.transaction(&Scope::Internal, |tx| {
                tx.resolve_account(&account, bound.as_deref(), &selector)
            }),
            Packet::BeginOrigin { id, frame, origin } => {
                let result = self.handle(Packet::Begin { id, frame })?;
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
                    seen: Default::default(),
                    changes: Vec::new(),
                    id,
                    frame,
                    failed: None,
                });
                Ok(Value::Null)
            }
            Packet::Call { id, call } => {
                let session = self
                    .session
                    .as_mut()
                    .filter(|s| s.id == id)
                    .ok_or_else(|| Fault::denied("unknown request transaction"))?;
                if let Some(e) = &session.failed {
                    return Err(e.clone());
                }
                let scope = Scope::Granted {
                    tenants: session
                        .frame
                        .tenants
                        .clone()
                        .unwrap_or_else(|| vec![session.frame.tenant.clone()]),
                    sets: session.frame.sets.clone(),
                };
                let mut tx = Transaction {
                    admission: self.admission.as_ref(),
                    origin: session.origin.clone(),
                    seen: (!session.frame.tenant.is_empty()).then_some(&mut session.seen),
                    changes: &mut session.changes,
                    outbox: self.outbox,
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
                let result = if session.frame.tenant.is_empty() {
                    let declared = self.policies.get(&call.entity).is_some_and(|p| {
                        p.entity["own"] == "OWN_HOLDER"
                            && p.methods.get("Memberships").unwrap_or(&Value::Null)["policy"]
                                .get("account")
                                .is_some()
                    });
                    if self
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
                        && session.frame.action == format!("/{}Service/Memberships", call.entity)
                        && call.method == "Memberships"
                    {
                        let mut reader = Transaction {
                            admission: None,
                            origin: None,
                            seen: None,
                            changes: tx.changes,
                            outbox: tx.outbox,
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
                    } else {
                        Err(Fault::denied(
                            "account method has no declared read of this entity",
                        ))
                    }
                } else {
                    tx.call(call)
                };
                session.failed = result.as_ref().err().cloned().or(tx.failed);
                result
            }
            Packet::Finish { id, commit } => {
                if !self.session.as_ref().is_some_and(|s| s.id == id) {
                    return Err(Fault::denied("unknown request transaction"));
                }
                let failed = self.session.as_ref().unwrap().failed.clone();
                let db = self.db.driver()?;
                if commit && failed.is_none() {
                    if let Err(err) = db.batch("COMMIT") {
                        db.batch("ROLLBACK")?;
                        self.session = None;
                        return Err(err);
                    }
                } else {
                    db.batch("ROLLBACK")?;
                }
                let session = self.session.take().expect("validated session");
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
                serde_json::to_value(event).map_err(|e| Fault::internal(e.to_string()))
            }
        }
    }
}
