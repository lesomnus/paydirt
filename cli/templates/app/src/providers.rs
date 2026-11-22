use super::{Database, Principal, tokens};
use connectrpc::{ConnectError, RequestContext};
use paydirt_server::authentication::{Config, Evidence, Verifier};
use paydirt_store::auth::Sessions;
use std::{collections::HashMap, sync::Arc, time::Duration};
#[derive(Clone)]
pub(super) struct Verified(pub Evidence);
pub(super) struct Providers {
    verifier: Verifier,
    bindings: HashMap<String, Principal>,
    sessions: Option<Sessions>,
}
const COOKIE: &str = "__Host-pd_session";
fn cookie(ctx: &RequestContext) -> Result<Option<&str>, ConnectError> {
    let mut found = None;
    for h in ctx.headers().get_all("cookie").iter() {
        for s in h
            .to_str()
            .map_err(|_| ConnectError::unauthenticated("invalid cookie"))?
            .split(';')
        {
            if let Some((name, key)) = s.trim().split_once('=')
                && name == COOKIE
            {
                if found.replace(key).is_some() {
                    return Err(ConnectError::unauthenticated("duplicate session cookie"));
                }
            }
        }
    }
    Ok(found)
}
impl Providers {
    pub(super) fn uses_session(&self,ctx:&RequestContext)->Result<bool,ConnectError>{Ok(self.sessions.is_some() && ctx.header("authorization").is_none() && cookie(ctx)?.is_some())}
    pub(super) async fn verify(
        &self,
        ctx: &RequestContext,
    ) -> Result<Option<Evidence>, ConnectError> {
        if ctx.header("authorization").is_none()
            && let Some(sessions) = &self.sessions
            && let Some(key) = cookie(ctx)?
        {
            let expected = ctx
                .headers()
                .get_all("payday-identity")
                .iter()
                .map(|h| {
                    h.to_str()
                        .map_err(|_| ConnectError::unauthenticated("invalid expected identity"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let session = sessions.read(key, expected).map_err(super::error)?;
            let e: Evidence = serde_json::from_str(&session.principal)
                .map_err(|_| ConnectError::internal("invalid stored session"))?;
            e.live()?;
            return Ok(Some(e));
        }
        self.verifier.resolve(ctx).await
    }
    pub(super) fn principal(&self, e: &Evidence, action: &str) -> Result<Principal, ConnectError> {
        let mut p =
            self.bindings.get(&e.binding).cloned().ok_or_else(|| {
                ConnectError::unauthenticated("identity has no application binding")
            })?;
        if e.actions
            .as_ref()
            .is_some_and(|v| !v.iter().any(|a| paydirt::tenant::covers(a, action)))
        {
            return Err(ConnectError::permission_denied(
                "method outside token grant",
            ));
        }
        let narrow = |current: &mut Option<Vec<Vec<u8>>>, remote: &Option<Vec<Vec<u8>>>| {
            if let Some(allowed) = remote {
                *current = Some(match current.take() {
                    Some(ids) => ids.into_iter().filter(|i| allowed.contains(i)).collect(),
                    None => allowed.clone(),
                })
            }
        };
        // An unqualified local identity has one tenant, even if the external
        // credential can name more. Intersect both boundaries.
        if e.tenants.is_some() && p.frame.tenants.is_none() {
            p.frame.tenants = Some(vec![p.frame.tenant.clone()])
        }
        narrow(&mut p.frame.tenants, &e.tenants);
        narrow(&mut p.frame.sets, &e.sets);
        Ok(p)
    }
}
impl Database {
    pub fn with_authentication(mut self, config: Config) -> Result<Self, String> {
        let verifier = Verifier::new(&config)?;
        let bindings =
            tokens(&serde_json::to_string(&config.bindings).map_err(|e| e.to_string())?)?;
        self.providers = Some(Arc::new(Providers {
            verifier,
            bindings,
            sessions: config.sessions.then(Sessions::default),
        }));
        Ok(self)
    }
    /// Called by an application's login handler after its normal authentication gate.
    pub fn login_cookie(
        &self,
        ctx: &RequestContext,
        identity: &str,
        ttl: Duration,
    ) -> Result<String, ConnectError> {
        let evidence = &ctx
            .extensions()
            .get::<Verified>()
            .ok_or_else(|| ConnectError::unauthenticated("login requires a verified provider"))?
            .0;
        if cookie(ctx)?.is_some() && ctx.header("authorization").is_none() {
            return Err(ConnectError::unauthenticated(
                "login requires fresh credentials",
            ));
        }
        let sessions = self
            .providers
            .as_ref()
            .and_then(|p| p.sessions.as_ref())
            .ok_or_else(|| ConnectError::failed_precondition("sessions are disabled"))?;
        evidence.live()?;
        let key = sessions
            .issue(
                serde_json::to_string(evidence)
                    .map_err(|_| ConnectError::internal("session encoding"))?,
                identity.into(),
                ttl,
            )
            .map_err(super::error)?;
        Ok(format!(
            "{COOKIE}={key}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={}",
            ttl.as_secs()
        ))
    }
    pub fn logout_cookie(&self, ctx: &RequestContext) -> Result<String, ConnectError> {
        if let Some(key) = cookie(ctx)?
            && let Some(s) = self.providers.as_ref().and_then(|p| p.sessions.as_ref())
        {
            s.revoke(key)
        }
        Ok(format!(
            "{COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0"
        ))
    }
    pub fn evidence<'a>(&self, ctx: &'a RequestContext) -> Option<&'a Evidence> {
        ctx.extensions().get::<Verified>().map(|v| &v.0)
    }
}
