use connectrpc::{ConnectError, RequestContext};
use paydirt_store::{Call, Fault, Frame, Packet};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

pub const MODEL: &str = include_str!("../generated/storage/model.json");
pub const POLICIES: &str = include_str!("../generated/storage/policies.json");

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Credential {
    Tenant(String),
    Account {
        #[serde(default)]
        topics: paydirt_store::TopicGrants,
        account: String,
        tenant: Option<String>,
        methods: Option<Vec<String>>,
    },
    Principal {
        #[serde(default)]
        topics: paydirt_store::TopicGrants,
        tenant: String,
        name: Option<String>,
        actor: Option<String>,
        caller: Option<String>,
        caller_tenant: Option<String>,
        methods: Option<Vec<String>>,
        tenants: Option<Vec<String>>,
        sets: Option<Vec<String>>,
    },
}

#[derive(Clone)]
struct Principal {
    name: Option<String>,
    frame: Frame,
    methods: Option<Vec<String>>,
}

fn tokens(config: &str) -> Result<HashMap<String, Principal>, String> {
    fn id(hex: &str, domain: u8) -> Result<Vec<u8>, String> {
        let bytes = hex::decode(hex).map_err(|e| e.to_string())?;
        if !paydirt::has_domain(&bytes, domain) {
            return Err(format!("invalid domain {domain} ID"));
        }
        Ok(bytes)
    }
    let credentials: HashMap<String, Credential> =
        serde_json::from_str(config).map_err(|e| e.to_string())?;
    credentials
        .into_iter()
        .map(|(token, c)| {
            if token.is_empty() {
                return Err("empty bearer token".into());
            }
            let topics = match &c {
                Credential::Tenant(_) => Default::default(),
                Credential::Principal { topics, .. } | Credential::Account { topics, .. } => {
                    topics.clone()
                }
            };
            let (tenant, actor, caller, caller_tenant, methods, tenants, sets, account, name) =
                match c {
                    Credential::Tenant(tenant) => {
                        (Some(tenant), None, None, None, None, None, None, None, None)
                    }
                    Credential::Account {
                        topics: _,
                        account,
                        tenant,
                        methods,
                    } => (
                        tenant,
                        None,
                        None,
                        None,
                        methods,
                        None,
                        None,
                        Some(account),
                        None,
                    ),
                    Credential::Principal {
                        topics: _,
                        name,
                        tenant,
                        actor,
                        caller,
                        caller_tenant,
                        methods,
                        tenants,
                        sets,
                    } => (
                        Some(tenant),
                        actor,
                        caller,
                        caller_tenant,
                        methods,
                        tenants,
                        sets,
                        None,
                        name,
                    ),
                };
            if caller.is_some() != caller_tenant.is_some() || (caller.is_some() && actor.is_none())
            {
                return Err("incomplete represented principal".into());
            }
            Ok((
                token,
                Principal {
                    frame: Frame {
                        topics,
                        tenant: tenant
                            .as_deref()
                            .map(|v| id(v, 1))
                            .transpose()?
                            .unwrap_or_default(),
                        account: account
                            .map(|v| {
                                paydirt::parse_id(&v)
                                    .filter(|id| *id != [0; 16])
                                    .map(|id| id.to_vec())
                                    .ok_or("invalid account ID")
                            })
                            .transpose()?,
                        tenants: tenants
                            .map(|v| v.iter().map(|t| id(t, 1)).collect::<Result<_, _>>())
                            .transpose()?,
                        sets: sets
                            .map(|v| {
                                v.iter()
                                    .map(|t| {
                                        let bytes = hex::decode(t).map_err(|e| e.to_string())?;
                                        if bytes.len() != 16
                                            || !paydirt::has_domain(&bytes, bytes[9])
                                        {
                                            return Err("invalid set ID".into());
                                        }
                                        Ok(bytes)
                                    })
                                    .collect::<Result<_, String>>()
                            })
                            .transpose()?,
                        actor: actor.as_deref().map(|v| id(v, 2)).transpose()?,
                        caller: caller.as_deref().map(|v| id(v, 2)).transpose()?,
                        caller_tenant: caller_tenant.as_deref().map(|v| id(v, 1)).transpose()?,
                        trace: Vec::new(),
                        action: String::new(),
                    },
                    methods,
                    name,
                },
            ))
        })
        .collect()
}

fn error(f: Fault) -> ConnectError {
    match f.code.as_str() {
        "invalid_argument" => ConnectError::invalid_argument(f.message),
        "already_exists" => ConnectError::already_exists(f.message),
        "failed_precondition" => ConnectError::failed_precondition(f.message),
        "aborted" => ConnectError::aborted(f.message),
        "not_found" => ConnectError::not_found(f.message),
        "permission_denied" => ConnectError::permission_denied(f.message),
        "unauthenticated" => ConnectError::unauthenticated(f.message),
        "unimplemented" => ConnectError::unimplemented(f.message),
        _ => ConnectError::internal("database operation failed"),
    }
}

#[derive(Clone)]
pub struct Database {
    principals: Arc<HashMap<String, Principal>>,
    lock: Arc<async_lock::Mutex<()>>,
    general_writes: bool,
    policy: Option<paydirt::rbac::Policy>,
    account_header: String,
    #[cfg(not(target_arch = "wasm32"))]
    queue: std::sync::mpsc::Sender<(
        Packet,
        futures_channel::oneshot::Sender<Result<Value, Fault>>,
    )>,
}

type Reply = Pin<Box<dyn Future<Output = Result<Value, ConnectError>> + Send>>;
#[derive(Clone)]
struct Session(String);
#[derive(Clone)]
struct PolicyRevision(u64);

impl Database {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(url: &str, config: &str) -> Result<Self, String> {
        let principals = Arc::new(tokens(config)?);
        let mut store =
            paydirt_store::Store::open(url, MODEL, POLICIES).map_err(|e| e.to_string())?;
        let (queue, events) = std::sync::mpsc::channel::<(
            Packet,
            futures_channel::oneshot::Sender<Result<Value, Fault>>,
        )>();
        std::thread::Builder::new()
            .name("paydirt-database".into())
            .spawn(move || {
                for (packet, reply) in events {
                    let _ = reply.send(store.handle(packet));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            principals,
            lock: Default::default(),
            queue,
            general_writes: false,
            policy: None,
            account_header: "paydirt-tenant".into(),
        })
    }
    #[cfg(target_arch = "wasm32")]
    pub fn worker(config: &str) -> Result<Self, String> {
        Ok(Self {
            principals: Arc::new(tokens(config)?),
            lock: Default::default(),
            general_writes: false,
            policy: None,
            account_header: "paydirt-tenant".into(),
        })
    }
    pub fn allow_general_writes(mut self, allow: bool) -> Self {
        self.general_writes = allow;
        self
    }
    pub fn with_policy(mut self, policy: paydirt::rbac::Policy) -> Self {
        self.policy = Some(policy);
        self
    }
    pub fn with_account_header(mut self, header: String) -> Result<Self, String> {
        if header.is_empty()
            || header.starts_with("grpc-")
            || header.ends_with("-bin")
            || !header
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err("invalid account tenant header".into());
        }
        self.account_header = header;
        Ok(self)
    }
    pub async fn authorize(
        &self,
        mut ctx: RequestContext,
        entity: &str,
        method: &str,
        public: bool,
        direct: bool,
        account_method: bool,
    ) -> Result<RequestContext, ConnectError> {
        let action = format!("/{entity}Service/{method}");
        if !self.general_writes && matches!(method, "Patch" | "Apply") {
            return Err(ConnectError::unimplemented(format!("{action} is closed")));
        }
        let Some(header) = ctx.headers().get("authorization") else {
            if public && !direct {
                return Ok(ctx);
            }
            return Err(ConnectError::unauthenticated("bearer token required"));
        };
        let principal = header
            .to_str()
            .ok()
            .and_then(|s| s.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
            .and_then(|(_, token)| self.principals.get(token))
            .ok_or_else(|| ConnectError::unauthenticated("unknown bearer token"))?;
        if principal.methods.as_ref().is_some_and(|allowed| {
            !allowed
                .iter()
                .any(|grant| paydirt::tenant::covers(grant, &action))
        }) {
            return Err(ConnectError::permission_denied(
                "method outside credential grant",
            ));
        }
        if direct && principal.frame.caller.is_some() {
            return Err(ConnectError::permission_denied(
                "direct credential required",
            ));
        }
        let mut frame = principal.frame.clone();
        let mut name = principal.name.clone();
        if let Some(account) = &frame.account {
            if account_method {
                if !frame.tenant.is_empty() {
                    return Err(ConnectError::permission_denied(
                        "credential is bound to a tenant",
                    ));
                }
                frame.actor = Some(account.clone());
            } else {
                let mut headers = ctx.headers().get_all(&self.account_header).iter();
                let selector = headers
                    .next()
                    .map(|v| v.to_str())
                    .transpose()
                    .map_err(|_| ConnectError::invalid_argument("invalid tenant selector"))?
                    .unwrap_or("");
                if headers.next().is_some() {
                    return Err(ConnectError::invalid_argument("multiple tenant selectors"));
                }
                let _guard = self.lock.lock_arc().await;
                #[derive(Deserialize)]
                struct Member {
                    actor: Vec<u8>,
                    tenant: Vec<u8>,
                    name: String,
                }
                let found = self
                    .send(Packet::ResolveAccount {
                        account: account.clone(),
                        bound: (!frame.tenant.is_empty()).then(|| frame.tenant.clone()),
                        selector: selector.to_string(),
                    })
                    .await?;
                let member: Member = serde_json::from_value(found)
                    .map_err(|_| ConnectError::internal("invalid membership"))?;
                frame.actor = Some(member.actor);
                frame.tenant = member.tenant;
                name = Some(member.name);
            }
        } else if account_method {
            return Err(ConnectError::failed_precondition(
                "an account makes this call as itself",
            ));
        }
        if let Some(policy) = &self.policy {
            let (revision, snapshot) = policy.pin();
            if !account_method && !name.as_deref().is_some_and(|n| snapshot.allows(n, &action)) {
                return Err(ConnectError::permission_denied(
                    "method outside holder roles",
                ));
            }
            ctx.extensions_mut().insert(PolicyRevision(revision));
        }
        frame.action = action;
        if let Some(trace) = ctx
            .headers()
            .get("traceparent")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split('-').nth(1))
            .filter(|s| s.len() == 32)
            .and_then(|v| hex::decode(v).ok())
        {
            frame.trace = trace;
        }
        ctx.extensions_mut().insert(frame);
        Ok(ctx)
    }
    pub async fn begin(&self, ctx: &mut RequestContext) -> Result<Option<Guard>, ConnectError> {
        let Some(frame) = ctx.extensions().get::<Frame>().cloned() else {
            return Ok(None);
        };
        let permit = self.lock.lock_arc().await;
        let id = hex::encode(paydirt::mint(0));
        let guard = Guard {
            db: self.clone(),
            id: Some(id.clone()),
            _permit: permit,
        };
        self.send(Packet::Begin {
            id: id.clone(),
            frame,
        })
        .await?;
        ctx.extensions_mut().insert(Session(id));
        Ok(Some(guard))
    }
    pub async fn rpc<I: Serialize, O: DeserializeOwned>(
        &self,
        ctx: &RequestContext,
        entity: &str,
        method: &str,
        request: &I,
    ) -> Result<O, ConnectError> {
        let id = &ctx
            .extensions()
            .get::<Session>()
            .ok_or_else(|| ConnectError::unauthenticated("no request transaction"))?
            .0;
        let call = Call {
            entity: entity.into(),
            method: method.into(),
            request: serde_json::to_value(request)
                .map_err(|_| ConnectError::invalid_argument("invalid request"))?,
        };
        let out = self
            .send(Packet::Call {
                id: id.clone(),
                call,
            })
            .await?;
        serde_json::from_value(out)
            .map_err(|_| ConnectError::internal("invalid response from storage"))
    }
    /// Write a declared domain event and the changed facts in this Core call's transaction.
    pub async fn event<E: buffa::Message + buffa::MessageName, I: Serialize>(
        &self,
        ctx: &RequestContext,
        entity: &str,
        kind: &str,
        payload: &E,
        before: &I,
        after: &I,
    ) -> Result<Value, ConnectError> {
        let value = buffa_types::google::protobuf::Any::pack(
            payload,
            format!("type.googleapis.com/{}", E::FULL_NAME),
        );
        self.rpc(
            ctx,
            entity,
            "$event",
            &serde_json::json!({"kind":kind,"value":value,"before":before,"after":after}),
        )
        .await
    }
    pub async fn state(
        &self,
        ctx: &RequestContext,
        entity: &str,
        reference: Value,
        at: &str,
        known: Option<&str>,
    ) -> Result<Value, ConnectError> {
        self.rpc(
            ctx,
            entity,
            "$state",
            &serde_json::json!({"ref":reference,"at":at,"known":known}),
        )
        .await
    }
    async fn read(
        &self,
        ctx: &RequestContext,
        entity: &str,
        op: &str,
        request: &Value,
    ) -> Result<Value, ConnectError> {
        if let (Some(policy), Some(revision)) =
            (&self.policy, ctx.extensions().get::<PolicyRevision>())
        {
            if policy.pin().0 != revision.0 {
                return Err(ConnectError::unauthenticated("policy changed; reconnect"));
            }
        }
        if let Some(frame) = ctx.extensions().get::<Frame>()
            && let Some(account) = &frame.account
            && !frame.tenant.is_empty()
        {
            let _guard = self.lock.lock_arc().await;
            let member = self
                .send(Packet::ResolveAccount {
                    account: account.clone(),
                    bound: Some(frame.tenant.clone()),
                    selector: String::new(),
                })
                .await?;
            if serde_json::to_value(&frame.actor).ok().as_ref() != member.get("actor") {
                return Err(ConnectError::unauthenticated("membership changed"));
            }
        }
        let mut ctx = ctx.clone();
        let guard = self
            .begin(&mut ctx)
            .await?
            .ok_or_else(|| ConnectError::unauthenticated("watch requires a caller"))?;
        let response = self.rpc(&ctx, entity, op, request).await;
        guard.finish(response.is_ok()).await?;
        response
    }

    pub async fn listen<I: Serialize, O: DeserializeOwned + Send + 'static>(
        &self,
        ctx: RequestContext,
        entity: &'static str,
        req: &I,
    ) -> connectrpc::ServiceResult<connectrpc::ServiceStream<O>> {
        let request = serde_json::to_value(req)
            .map_err(|_| ConnectError::invalid_argument("invalid listen request"))?;
        let start = self.read(&ctx, entity, "$listen", &request).await?;
        let db = self.clone();
        let stream = async_stream::try_stream! {
            let mut old=start["revisions"].clone();
            yield serde_json::from_value(serde_json::json!({"ready":{"readerId":start["readerId"]}})).map_err(|_|ConnectError::internal("invalid listen reply"))?;
            loop{
                futures_timer::Delay::new(std::time::Duration::from_millis(80)).await;
                let batch=db.read(&ctx,entity,"$listen",&request).await?;
                let changed:Vec<_>=batch["topics"].as_array().into_iter().flatten().enumerate().filter(|(i,_)|old[*i]!=batch["revisions"][*i]).map(|(_,v)|v).collect();
                if !changed.is_empty(){yield serde_json::from_value(serde_json::json!({"invalidated":{"topics":changed}})).map_err(|_|ConnectError::internal("invalid listen change"))?;}
                old=batch["revisions"].clone();
            }
        };
        connectrpc::Response::stream_ok(stream)
    }
    pub async fn watch<I: Serialize, O: DeserializeOwned + Send + 'static>(
        &self,
        ctx: RequestContext,
        entity: &'static str,
        request: &I,
    ) -> connectrpc::ServiceResult<connectrpc::ServiceStream<O>> {
        let request = serde_json::to_value(request)
            .map_err(|_| ConnectError::invalid_argument("invalid watch request"))?;
        let start = self.read(&ctx, entity, "$watch_start", &request).await?;
        let db = self.clone();
        let stream = async_stream::try_stream! {
            let mut seen=std::collections::HashSet::new();
            for page in start["pages"].as_array().into_iter().flatten(){
                for item in page["items"].as_array().into_iter().flatten(){seen.insert(item["id"].as_str().unwrap().to_owned());}
                yield serde_json::from_value(page.clone()).map_err(|_|ConnectError::internal("invalid watch snapshot"))?;
            }
            let mut poll=serde_json::json!({"ids":start["ids"],"revision":start["revision"]});
            loop {
                let batch=db.read(&ctx,entity,"$watch_poll",&poll).await?;
                poll["revision"]=batch["revision"].clone();
                let mut items=Vec::new();
                for item in batch["items"].as_array().into_iter().flatten(){
                    let id=item["id"].as_str().unwrap();
                    if item.get("value").is_some(){seen.insert(id.to_owned());}
                    else if !seen.remove(id){continue;}
                    items.push(item);
                }
                if !items.is_empty(){
                    yield serde_json::from_value(serde_json::json!({"items":items})).map_err(|_|ConnectError::internal("invalid watch response"))?;
                }
                futures_timer::Delay::new(std::time::Duration::from_millis(80)).await;
            }
        };
        connectrpc::Response::stream_ok(stream)
    }
    // Enqueue synchronously. A dropped RPC always queues its rollback before the lock passes to the next call.
    #[cfg(not(target_arch = "wasm32"))]
    fn send(&self, packet: Packet) -> Reply {
        let (tx, rx) = futures_channel::oneshot::channel();
        let _ = self.queue.send((packet, tx));
        Box::pin(async move {
            rx.await
                .map_err(|_| ConnectError::unavailable("database stopped"))?
                .map_err(error)
        })
    }
    #[cfg(target_arch = "wasm32")]
    fn send(&self, packet: Packet) -> Reply {
        use wasm_bindgen::prelude::*;
        #[wasm_bindgen]
        extern "C" {
            #[wasm_bindgen(js_name=paydirtQuery,catch)]
            fn query(request: &str) -> Result<js_sys::Promise, JsValue>;
        }
        let pending = serde_json::to_string(&packet)
            .map_err(|_| ConnectError::internal("invalid worker request"))
            .and_then(|p| {
                query(&p).map_err(|_| ConnectError::unavailable("database worker not connected"))
            })
            .map(|p| send_wrapper::SendWrapper::new(wasm_bindgen_futures::JsFuture::from(p)));
        Box::pin(async move {
            let text = pending?
                .await
                .map_err(|_| ConnectError::unavailable("database worker stopped"))?
                .as_string()
                .ok_or_else(|| ConnectError::internal("invalid worker reply"))?;
            serde_json::from_str::<Result<Value, Fault>>(&text)
                .map_err(|_| ConnectError::internal("invalid worker reply"))?
                .map_err(error)
        })
    }
}

pub struct Guard {
    db: Database,
    id: Option<String>,
    _permit: async_lock::MutexGuardArc<()>,
}
impl Guard {
    pub async fn finish(mut self, commit: bool) -> Result<(), ConnectError> {
        let id = self.id.take().expect("active transaction");
        self.db
            .send(Packet::Finish { id, commit })
            .await
            .map(|_| ())
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            drop(self.db.send(Packet::Finish { id, commit: false }));
        }
    }
}
