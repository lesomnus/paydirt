use paydirt_robot_protocol::{Failure, Reply, Request};

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct Backend(std::sync::Arc<std::sync::Mutex<paydirt_storage::Store>>);

#[cfg(not(target_arch = "wasm32"))]
impl Backend {
    pub fn open(url: &str) -> Result<Self, String> {
        Ok(Self(std::sync::Arc::new(std::sync::Mutex::new(
            paydirt_storage::Store::open(url)?,
        ))))
    }
    pub async fn request(&self, req: Request) -> Reply {
        let db = self.0.clone();
        tokio::task::spawn_blocking(move || db.lock().map_err(|_| Failure::Internal)?.execute(req))
            .await
            .map_err(|_| Failure::Internal)?
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
pub struct Backend;
#[cfg(target_arch = "wasm32")]
impl Backend {
    pub async fn request(&self, req: Request) -> Reply {
        use wasm_bindgen::prelude::*;
        #[wasm_bindgen]
        extern "C" {
            #[wasm_bindgen(js_name = paydirtQuery, catch)]
            fn query(payload: &str) -> Result<js_sys::Promise, JsValue>;
        }
        let payload = serde_json::to_string(&req).map_err(|_| Failure::Internal)?;
        let pending = query(&payload).map_err(|_| Failure::Unavailable)?;
        // PortServer always polls these futures on the dedicated service Worker.
        let text = send_wrapper::SendWrapper::new(wasm_bindgen_futures::JsFuture::from(pending))
            .await
            .map_err(|_| Failure::Unavailable)?
            .as_string()
            .ok_or(Failure::Internal)?;
        serde_json::from_str(&text).map_err(|_| Failure::Internal)?
    }
}
