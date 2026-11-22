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
    Principal {
        tenant: String,
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
            let (tenant, actor, caller, caller_tenant, methods, tenants, sets) = match c {
                Credential::Tenant(tenant) => (tenant, None, None, None, None, None, None),
                Credential::Principal {
                    tenant,
                    actor,
                    caller,
                    caller_tenant,
                    methods,
                    tenants,
                    sets,
                } => (tenant, actor, caller, caller_tenant, methods, tenants, sets),
            };
            if caller.is_some() != caller_tenant.is_some() || (caller.is_some() && actor.is_none())
            {
                return Err("incomplete represented principal".into());
            }
            Ok((
                token,
                Principal {
                    frame: Frame {
                        tenant: id(&tenant, 1)?,
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
    #[cfg(not(target_arch = "wasm32"))]
    queue: std::sync::mpsc::Sender<(
        Packet,
        futures_channel::oneshot::Sender<Result<Value, Fault>>,
    )>,
}

type Reply = Pin<Box<dyn Future<Output = Result<Value, ConnectError>> + Send>>;
#[derive(Clone)]
struct Session(String);

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
        })
    }
    #[cfg(target_arch = "wasm32")]
    pub fn worker(config: &str) -> Result<Self, String> {
        Ok(Self {
            principals: Arc::new(tokens(config)?),
            lock: Default::default(),
            general_writes: false,
        })
    }
    pub fn allow_general_writes(mut self, allow: bool) -> Self {
        self.general_writes = allow;
        self
    }
    pub fn authorize(
        &self,
        mut ctx: RequestContext,
        entity: &str,
        method: &str,
        public: bool,
        direct: bool,
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
    async fn read(
        &self,
        ctx: &RequestContext,
        entity: &str,
        op: &str,
        request: &Value,
    ) -> Result<Value, ConnectError> {
        let mut ctx = ctx.clone();
        let guard = self
            .begin(&mut ctx)
            .await?
            .ok_or_else(|| ConnectError::unauthenticated("watch requires a caller"))?;
        let response = self.rpc(&ctx, entity, op, request).await;
        guard.finish(response.is_ok()).await?;
        response
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
