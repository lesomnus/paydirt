//! Revocable session credentials for an application-owned sign-in endpoint.
use crate::{Fault, Result};
use chrono::{DateTime, Utc};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Clone)]
pub struct Sessions(Arc<Mutex<HashMap<String, Session>>>);
#[derive(Clone)]
pub struct Session {
    pub principal: String,
    pub identity: String,
    expires: DateTime<Utc>,
}
impl Default for Sessions {
    fn default() -> Self {
        Self(Default::default())
    }
}
impl Sessions {
    pub fn issue(&self, principal: String, identity: String, ttl: Duration) -> Result<String> {
        if identity.is_empty() || principal.is_empty() || ttl.is_zero() {
            return Err(Fault::invalid(
                "principal, identity and session lifetime required",
            ));
        }
        let now = Utc::now();
        let expires = now
            .checked_add_signed(
                chrono::TimeDelta::from_std(ttl)
                    .map_err(|_| Fault::invalid("invalid session lifetime"))?,
            )
            .ok_or_else(|| Fault::invalid("invalid session lifetime"))?;
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).map_err(|e| Fault::internal(e.to_string()))?;
        let key = hex::encode(secret);
        let mut sessions = self.0.lock().unwrap();
        sessions.retain(|_, v| v.expires > now);
        sessions.insert(
            key.clone(),
            Session {
                principal,
                identity,
                expires,
            },
        );
        Ok(key)
    }
    pub fn revoke(&self, key: &str) {
        self.0.lock().unwrap().remove(key);
    }
    pub fn read<'a>(
        &self,
        key: &str,
        expected: impl IntoIterator<Item = &'a str>,
    ) -> Result<Session> {
        let mut s = self.0.lock().unwrap();
        let found = s.get(key).filter(|v| v.expires > Utc::now()).cloned();
        let Some(found) = found else {
            s.remove(key);
            return Err(Fault::unauthenticated("session expired"));
        };
        if expected.into_iter().any(|v| v != found.identity) {
            return Err(Fault::unauthenticated(
                "session belongs to another identity",
            ));
        }
        Ok(found)
    }
}
