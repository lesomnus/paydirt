//! App-verified authority to represent a person. Header values are claims, not proof.
use super::{Database, Principal};
use chrono::{DateTime, Utc};
use connectrpc::{ConnectError, RequestContext, ServiceStream};
use futures_util::{FutureExt, StreamExt};
use paydirt_store::Frame;
use serde_json::Value;
use std::{sync::Arc, time::Duration};

#[derive(Clone, Debug, Default)]
pub struct Authentication {
    pub method: String,
    pub at: Option<DateTime<Utc>>,
}
impl Authentication {
    pub fn fresh(&self, now: DateTime<Utc>, window: Duration) -> bool {
        !window.is_zero()
            && self.at.is_some_and(|at| {
                now.signed_duration_since(at)
                    .to_std()
                    .is_ok_and(|age| age <= window)
            })
    }
}
#[derive(Clone, Debug)]
pub struct Parties {
    pub actor: Authentication,
    pub caller: Option<Authentication>,
    pub recorded: Option<DateTime<Utc>>,
}
pub struct Claim<'a> {
    pub caller: &'a Frame,
    pub authentication: &'a Authentication,
    pub reference: &'a str,
    pub subject: [u8; 16],
    pub method: &'a str,
    pub request: Option<&'a Value>,
    pub recorded: Option<DateTime<Utc>>,
}
#[derive(Clone)]
pub struct Lease {
    pub until: DateTime<Utc>,
    pub active: Arc<dyn Fn() -> bool + Send + Sync>,
}
impl Lease {
    pub fn check(&self) -> Result<(), ConnectError> {
        if self.until <= Utc::now() || !(self.active)() {
            return Err(ConnectError::permission_denied(
                "presence expired or revoked",
            ));
        }
        Ok(())
    }
}
pub struct Person {
    pub frame: Frame,
    pub name: Option<String>,
    pub methods: Option<Vec<String>>,
    pub authentication: Authentication,
    pub lease: Lease,
}
pub type Verify = dyn Fn(Claim<'_>) -> Result<Person, ConnectError> + Send + Sync;
#[derive(Clone)]
struct Request(Value);
fn header<'a>(ctx: &'a RequestContext, key: &str) -> Result<Option<&'a str>, ConnectError> {
    let mut all = ctx.headers().get_all(key).iter();
    let value = all
        .next()
        .map(|h| h.to_str())
        .transpose()
        .map_err(|_| denied())?;
    if all.next().is_some() || value.is_some_and(|s| s.is_empty() || s.len() > 512) {
        return Err(denied());
    }
    Ok(value)
}
fn denied() -> ConnectError {
    ConnectError::permission_denied("presence claim refused")
}
impl Database {
    pub fn with_presence(
        mut self,
        verify: impl Fn(Claim<'_>) -> Result<Person, ConnectError> + Send + Sync + 'static,
    ) -> Self {
        self.presence = Some(Arc::new(verify));
        self
    }
    pub fn presence_request<I: serde::Serialize>(
        mut ctx: RequestContext,
        req: impl FnOnce() -> I,
    ) -> Result<RequestContext, ConnectError> {
        if ctx.header("payday-presence").is_some() {
            ctx.extensions_mut()
                .insert(Request(serde_json::to_value(req()).map_err(|_| denied())?));
        }
        Ok(ctx)
    }
    pub(super) fn represented(
        &self,
        ctx: &mut RequestContext,
        device: Principal,
        method: &str,
        direct: bool,
    ) -> Result<Principal, ConnectError> {
        let mut auth = Authentication {
            method: "credential".into(),
            at: None,
        };
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(e) = self.evidence(ctx) {
            auth = Authentication {
                method: e.method.clone(),
                at: e.authenticated_at,
            };
        }
        let presence = header(ctx, "payday-presence")?;
        let subject = header(ctx, "payday-subject")?;
        let recorded = header(ctx, "payday-recorded")?
            .map(|t| t.parse::<DateTime<Utc>>().map_err(|_| denied()))
            .transpose()?;
        if presence.is_none() && subject.is_none() && recorded.is_none() {
            ctx.extensions_mut().insert(Parties {
                actor: auth,
                caller: None,
                recorded: None,
            });
            return Ok(device);
        }
        let subject = subject
            .and_then(paydirt::parse_id)
            .filter(|id| *id != [0; 16])
            .ok_or_else(denied)?;
        if direct
            || device.frame.caller.is_some()
            || device.frame.tenant.is_empty()
            || device.frame.actor.is_none()
        {
            return Err(denied());
        }
        if let Some(policy) = &self.policy
            && !device
                .name
                .as_deref()
                .is_some_and(|n| policy.pin().1.allows(n, method))
        {
            return Err(denied());
        }
        let who = self.presence.as_ref().ok_or_else(denied)?(Claim {
            caller: &device.frame,
            authentication: &auth,
            reference: presence.ok_or_else(denied)?,
            subject,
            method,
            request: ctx.extensions().get::<Request>().map(|v| &v.0),
            recorded,
        })?;
        if who.frame.actor.as_deref() != Some(subject.as_slice())
            || who.frame.caller.is_some()
            || who.frame.tenant.is_empty()
            || who.frame.account.is_some()
        {
            return Err(denied());
        }
        if who
            .methods
            .as_ref()
            .is_some_and(|v| !v.iter().any(|m| paydirt::tenant::covers(m, method)))
        {
            return Err(denied());
        }
        who.lease.check()?;
        let mut frame = who.frame;
        let tenants = device
            .frame
            .tenants
            .as_deref()
            .unwrap_or(std::slice::from_ref(&device.frame.tenant));
        if !tenants.contains(&frame.tenant) {
            return Err(denied());
        }
        frame.tenants = Some(
            frame
                .tenants
                .take()
                .unwrap_or_else(|| vec![frame.tenant.clone()])
                .into_iter()
                .filter(|t| tenants.contains(t))
                .collect(),
        );
        if let Some(device_sets) = device.frame.sets {
            frame.sets = Some(
                frame
                    .sets
                    .take()
                    .map(|sets| {
                        sets.into_iter()
                            .filter(|s| device_sets.contains(s))
                            .collect()
                    })
                    .unwrap_or(device_sets),
            )
        }
        frame.presence = presence.map(str::to_owned);
        frame.caller = device.frame.actor;
        frame.caller_row = device.frame.row;
        frame.caller_tenant = Some(device.frame.tenant);
        ctx.extensions_mut().insert(who.lease);
        ctx.extensions_mut().insert(Parties {
            actor: who.authentication,
            caller: Some(auth),
            recorded,
        });
        Ok(Principal {
            name: who.name,
            frame,
            methods: who.methods,
        })
    }
    pub fn enforce_presence<T: Send + 'static>(
        ctx: &RequestContext,
        mut upstream: ServiceStream<T>,
    ) -> ServiceStream<T> {
        let Some(lease) = ctx.extensions().get::<Lease>().cloned() else {
            return upstream;
        };
        Box::pin(async_stream::try_stream! {loop{
            lease.check()?;
            let next=upstream.next().fuse();
            let tick=futures_timer::Delay::new(Duration::from_millis(100)).fuse();
            futures_util::pin_mut!(next,tick);
            let message=futures_util::select_biased! {v=next=>v,_=tick=>continue};
            match message{None=>break,Some(v)=>{lease.check()?;yield v?;}}
        }})
    }
}
