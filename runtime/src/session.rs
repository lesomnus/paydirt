use crate::{Call, Fault, Result, Scope, Store, Transaction};
use paydirt_model::Dialect;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Established by the host's credential resolver before calling the application.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub tenant: Vec<u8>,
    #[serde(default)]
    pub actor: Option<Vec<u8>>,
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
    Begin { id: String, frame: Frame },
    Call { id: String, call: Call },
    Finish { id: String, commit: bool },
}
pub(crate) struct Session {
    pub id: String,
    pub frame: Frame,
    pub failed: Option<Fault>,
}

impl Store {
    /// The server serializes transactions, including across its private Worker port.
    /// Only a session opened on this connection can issue operations or finish it.
    pub fn handle(&mut self, packet: Packet) -> Result<Value> {
        match packet {
            Packet::Begin { id, frame } => {
                if self.session.is_some() {
                    return Err(Fault::aborted("another request owns this connection"));
                }
                if !paydirt::has_domain(&frame.tenant, 1) || id.is_empty() {
                    return Err(Fault::unauthenticated("invalid request frame"));
                }
                let db = self.db.driver()?;
                db.batch(if db.dialect() == Dialect::Sqlite {
                    "BEGIN IMMEDIATE"
                } else {
                    "BEGIN"
                })?;
                self.session = Some(Session {
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
                let scope = Scope::Tenant(session.frame.tenant.clone());
                let mut tx = Transaction {
                    db: self.db.driver()?,
                    policies: &self.policies,
                    model: &self.model,
                    scope: &scope,
                    frame: Some(&session.frame),
                    failed: None,
                };
                let result = tx.call(call);
                session.failed = tx.failed;
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
                self.session = None;
                if commit && let Some(err) = failed {
                    return Err(err);
                }
                Ok(Value::Null)
            }
        }
    }
}
