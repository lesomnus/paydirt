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
    ) -> Result<AppPort, JsError> {
        let epoch = NonZeroU32::new(epoch).ok_or_else(|| JsError::new("zero epoch"))?;
        let db = Database::worker(principals)
            .map_err(|e| JsError::new(&e))?
            .allow_general_writes(general_writes);
        Ok(Self {
            _server: drpc_rs::PortServer::bind(port, epoch, router(db, name)),
        })
    }
}
