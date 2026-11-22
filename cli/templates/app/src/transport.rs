use connectrpc::{ConnectError, RequestContext};
use paydirt::tenant::TenantTokens;
use paydirt_store::{Call, Fault, Scope};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

pub const MODEL: &str = include_str!("../generated/storage/model.json");
pub const POLICIES: &str = include_str!("../generated/storage/policies.json");

#[derive(Clone)]
pub struct Database {
    principals: Arc<TenantTokens>,
    #[cfg(not(target_arch = "wasm32"))]
    store: Arc<std::sync::Mutex<paydirt_store::Store>>,
}
fn tokens(config: &str) -> Result<Arc<TenantTokens>, String> {
    let pairs: HashMap<String, String> = serde_json::from_str(config).map_err(|e| e.to_string())?;
    let tenants = pairs
        .into_iter()
        .map(|(token, hex)| Ok((token, hex::decode(hex).map_err(|e| e.to_string())?)))
        .collect::<Result<_, String>>()?;
    Ok(Arc::new(TenantTokens::new(tenants, 1)?))
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
impl Database {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(url: &str, config: &str) -> Result<Self, String> {
        Ok(Self {
            principals: tokens(config)?,
            store: Arc::new(std::sync::Mutex::new(
                paydirt_store::Store::open(url, MODEL, POLICIES).map_err(|e| e.to_string())?,
            )),
        })
    }
    #[cfg(target_arch = "wasm32")]
    pub fn worker(config: &str) -> Result<Self, String> {
        Ok(Self {
            principals: tokens(config)?,
        })
    }
    pub async fn rpc<I: Serialize, O: DeserializeOwned>(
        &self,
        ctx: &RequestContext,
        entity: &str,
        method: &str,
        request: &I,
    ) -> Result<O, ConnectError> {
        let auth = ctx
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| self.principals.resolve(s))
            .ok_or_else(|| ConnectError::unauthenticated("unknown bearer token"))?;
        let call = Call {
            entity: entity.into(),
            method: method.into(),
            request: serde_json::to_value(request)
                .map_err(|_| ConnectError::invalid_argument("invalid request"))?,
        };
        let out = self.execute(Scope::Tenant(auth.to_vec()), call).await?;
        serde_json::from_value(out)
            .map_err(|_| ConnectError::internal("invalid response from storage"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    async fn execute(&self, scope: Scope, call: Call) -> Result<Value, ConnectError> {
        let db = self.store.clone();
        tokio::task::spawn_blocking(move || {
            db.lock()
                .map_err(|_| Fault::internal("database lock poisoned"))?
                .call(&scope, call)
        })
        .await
        .map_err(|_| ConnectError::internal("database thread stopped"))?
        .map_err(error)
    }
    #[cfg(target_arch = "wasm32")]
    async fn execute(&self, scope: Scope, call: Call) -> Result<Value, ConnectError> {
        use wasm_bindgen::prelude::*;
        #[wasm_bindgen]
        extern "C" {
            #[wasm_bindgen(js_name=paydirtQuery,catch)]
            fn query(request: &str) -> Result<js_sys::Promise, JsValue>;
        }
        let request = serde_json::to_string(&(scope, call))
            .map_err(|_| ConnectError::internal("invalid worker request"))?;
        let pending = query(&request)
            .map_err(|_| ConnectError::unavailable("database worker not connected"))?;
        let text = send_wrapper::SendWrapper::new(wasm_bindgen_futures::JsFuture::from(pending))
            .await
            .map_err(|_| ConnectError::unavailable("database worker stopped"))?
            .as_string()
            .ok_or_else(|| ConnectError::internal("invalid worker reply"))?;
        serde_json::from_str::<Result<Value, Fault>>(&text)
            .map_err(|_| ConnectError::internal("invalid worker reply"))?
            .map_err(error)
    }
}
