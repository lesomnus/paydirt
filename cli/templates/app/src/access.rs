use super::{Database, Frame, Packet};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
};
use connectrpc::{CodecFormat, ConnectError, Encodable, RequestContext, ServiceStream};
use futures_util::StreamExt;
use paydirt_store::{access::Observation, audit_content::Options};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    sync::{Arc, LazyLock, Mutex},
};
pub static OPTIONS: LazyLock<Options> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../trail.json")).expect("trail configuration")
});
#[derive(Clone, Default)]
pub struct Attempt {
    frame: Arc<Mutex<Option<Frame>>>,
    written: Arc<Mutex<BTreeSet<String>>>,
    pub(super) refused: Arc<Mutex<std::collections::BTreeMap<String, BTreeSet<String>>>>,
    request: Value,
    reason: Option<String>,
    input: &'static str,
    output: &'static str,
}
impl Attempt {
    pub fn start(
        ctx: &mut RequestContext,
        input: &'static str,
        output: &'static str,
        action: &str,
        download: bool,
        request: impl FnOnce() -> Result<Value, ConnectError>,
    ) -> Result<Self, ConnectError> {
        if !OPTIONS.access {
            return Ok(Default::default());
        }
        let reason = if download || OPTIONS.downloads.contains(action) {
            let raw = ctx.header("payday-reason-bin").ok_or_else(|| {
                ConnectError::invalid_argument("download requires payday-reason-bin")
            })?;
            let bytes = STANDARD
                .decode(raw)
                .or_else(|_| STANDARD_NO_PAD.decode(raw))
                .map_err(|_| ConnectError::invalid_argument("invalid binary reason"))?;
            let reason = std::str::from_utf8(&bytes)
                .map_err(|_| ConnectError::invalid_argument("reason must be UTF-8"))?
                .trim();
            if reason.is_empty() || reason.len() > 1024 {
                return Err(ConnectError::invalid_argument(
                    "expected a reason of up to 1024 bytes",
                ));
            }
            Some(reason.into())
        } else {
            None
        };
        let call = Self {
            input,
            output,
            reason,
            request: request()?,
            ..Default::default()
        };
        ctx.extensions_mut().insert(call.clone());
        Ok(call)
    }
    pub(super) fn frame(ctx: &RequestContext, f: &Frame, action: &str) {
        if let Some(attempt) = ctx.extensions().get::<Self>() {
            let mut f = f.clone();
            f.action = action.into();
            if let Some(peer) = ctx.extensions().get::<std::net::SocketAddr>() {
                f.address = Some(peer.ip().to_string());
            }
            *attempt.frame.lock().unwrap() = Some(f);
        }
    }
    pub(super) fn committed(&self, receipt: &Value) {
        let mut subjects = self.written.lock().unwrap();
        if let Some(ids) = receipt["writtenSubjects"].as_array() {
            subjects.extend(ids.iter().filter_map(Value::as_str).map(str::to_owned));
        } else {
            for change in receipt["changes"].as_array().into_iter().flatten() {
                if let Some(id) = change["key"].as_str().and_then(paydirt::parse_id) {
                    subjects.insert(STANDARD.encode(id));
                }
            }
        }
    }
    async fn record(
        &self,
        db: &Database,
        answer: Option<Value>,
        result: &str,
    ) -> Result<(), ConnectError> {
        let Some(frame) = self.frame.lock().unwrap().clone() else {
            return Ok(());
        };
        let observation = Observation {
            frame,
            request: self.request.clone(),
            input: self.input.into(),
            output: self.output.into(),
            answer,
            result: result.into(),
            reason: self.reason.clone(),
            written: self.written.lock().unwrap().clone(),
        };
        let _permit = db.lock.lock_arc().await;
        let ms = OPTIONS.access_flush_ms.unwrap_or(1000);
        let buffered = ms > 0
            && observation.reason.is_none()
            && !observation
                .frame
                .address
                .as_deref()
                .is_some_and(|a| a.starts_with("shell:"));
        let refused = std::mem::take(&mut *self.refused.lock().unwrap());
        let packet = if !refused.is_empty() {
            Packet::RefusedWrites {
                observation,
                subjects: refused,
                buffered,
            }
        } else if buffered {
            Packet::BufferObservation(observation)
        } else {
            Packet::Observe(observation)
        };
        let result = db
            .send(packet)
            .await
            .map_err(|_| ConnectError::unavailable("access record could not be written"))?;
        if buffered && result.as_u64().unwrap_or_default() > 0 {
            db.schedule_access_flush(ms);
        }
        Ok(())
    }
    pub async fn failed(&self, db: &Database, err: ConnectError) -> ConnectError {
        let _ = self.record(db, None, err.code.as_str()).await;
        err
    }
    pub fn output<'a, M: buffa::Message, R: Encodable<M>>(
        &'a self,
        db: &'a Database,
        body: &R,
    ) -> impl std::future::Future<Output = Result<(), ConnectError>> + Send + use<'a, M, R> {
        let doc = OPTIONS
            .access
            .then(|| {
                serde_json::from_slice(&body.encode(CodecFormat::Json)?)
                    .map_err(|_| ConnectError::internal("invalid access response"))
            })
            .transpose();
        async move {
            if let Some(doc) = doc? {
                self.record(db, Some(doc), "ok").await?
            }
            Ok(())
        }
    }
    pub fn stream<M: buffa::Message, R: Encodable<M> + Send + 'static>(
        self,
        db: Database,
        mut body: ServiceStream<R>,
    ) -> ServiceStream<R> {
        if !OPTIONS.access {
            return body;
        }
        Box::pin(
            async_stream::try_stream! {while let Some(item)=body.next().await{match item {Ok(v)=>{self.output::<M,_>(&db,&v).await?;yield v},Err(e)=>Err(self.failed(&db,e).await)?}}},
        )
    }
}

impl Database {
    fn schedule_access_flush(&self, ms: u64) {
        use std::sync::atomic::Ordering;
        if self.flushing_access.swap(true, Ordering::AcqRel) {
            return;
        }
        let db = self.clone();
        let job = async move {
            let mut delay = ms;
            loop {
                futures_timer::Delay::new(std::time::Duration::from_millis(delay)).await;
                let _permit = db.lock.lock_arc().await;
                if db.send(Packet::FlushAccess).await.is_ok() {
                    db.flushing_access.store(false, Ordering::Release);
                    break;
                }
                delay = delay.saturating_mul(2).clamp(100, 60_000);
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        tokio::spawn(job);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(job);
    }
    pub async fn flush_access(&self) -> Result<(), ConnectError> {
        let _permit = self.lock.lock_arc().await;
        self.send(Packet::FlushAccess).await.map(|_| ())
    }
}
