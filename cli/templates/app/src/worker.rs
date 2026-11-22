use crate::{router, transport::Database};
use std::num::NonZeroU32;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct AppPort {
    _server: drpc_rs::PortServer,
}
#[wasm_bindgen]
impl AppPort {
    #[wasm_bindgen(constructor)]
    pub fn new(
        port: web_sys::MessagePort,
        epoch: u32,
        principals: &str,
        name: String,
        general_writes: bool,
        policy: Option<String>,
        account_header: Option<String>,
    ) -> Result<AppPort, JsError> {
        let epoch = NonZeroU32::new(epoch).ok_or_else(|| JsError::new("zero epoch"))?;
        let mut db = Database::worker(principals)
            .map_err(|e| JsError::new(&e))?
            .allow_general_writes(general_writes);
        if let Some(doc) = policy {
            let rules = serde_json::from_str(&doc).map_err(|e| JsError::new(&e.to_string()))?;
            let snapshot = paydirt::rbac::Snapshot::new(rules).map_err(|e| JsError::new(&e))?;
            db = db.with_policy(paydirt::rbac::Policy::new(snapshot));
        }
        if let Some(header) = account_header {
            db = db
                .with_account_header(header)
                .map_err(|e| JsError::new(&e))?;
        }
        Ok(Self {
            _server: drpc_rs::PortServer::bind(port, epoch, router(db, name)),
        })
    }
}
