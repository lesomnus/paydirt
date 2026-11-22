//! A committed mutation names the resource to re-read. Consumers apply their
//! own current access policy before sending anything to a client.
use futures_channel::mpsc;
use futures_util::{StreamExt, future::BoxFuture, stream::BoxStream};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};

#[cfg(feature = "watch-nats")]
pub mod nats;
#[cfg(feature = "watch-postgres")]
pub mod postgres;
#[cfg(any(feature = "watch-nats", feature = "watch-postgres"))]
mod wire;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    #[serde(rename = "a", default, skip_serializing_if = "String::is_empty")]
    pub actor: String,
    #[serde(rename = "t", default, skip_serializing_if = "String::is_empty")]
    pub tenant: String,
    #[serde(rename = "m", default)]
    pub method: String,
    #[serde(rename = "c")]
    pub changes: Vec<Change>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Change {
    #[serde(rename = "b")]
    pub by: String,
    #[serde(rename = "k")]
    pub key: String,
    #[serde(rename = "m", default, skip_serializing_if = "String::is_empty")]
    pub method: String,
    #[serde(rename = "q", default)]
    pub routes: Option<Vec<String>>,
    #[serde(rename = "s", default, skip_serializing_if = "String::is_empty")]
    pub signal: String,
    #[serde(rename = "r", default, skip_serializing_if = "String::is_empty")]
    pub resource: String,
}
pub fn service_of(method: &str) -> &str {
    method.rfind('/').map_or(method, |n| &method[..=n])
}
pub fn route_key(parts: &[&str]) -> String {
    // Go's encoding/json escapes these characters even inside a JSON string.
    let json = serde_json::to_string(parts)
        .expect("string tuple")
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    hex::encode(Sha256::digest(json.as_bytes()))
}
#[derive(Clone, Default)]
pub struct Interest {
    pub service: String,
    pub keys: Vec<String>,
    pub routes: Vec<String>,
}
impl Interest {
    pub fn accepts(&self, event: &Event) -> bool {
        event.changes.iter().any(|c| {
            let service = service_of(&c.by);
            (self.service.is_empty() || service.is_empty() || self.service == service)
                && (self.keys.is_empty()
                    || c.key.is_empty()
                    || c.key == "00000000-0000-0000-0000-000000000000"
                    || self.keys.contains(&c.key))
                && (self.routes.is_empty()
                    || c.routes
                        .as_ref()
                        .is_none_or(|r| r.iter().any(|v| self.routes.contains(v))))
        })
    }
}
pub type Result<T> = std::result::Result<T, String>;
pub type Subscription = BoxStream<'static, Result<Arc<Event>>>;
pub trait Broker: Send + Sync {
    /// Successful return means the interest is installed, even before next() is called.
    fn subscribe(&self, interest: Interest) -> BoxFuture<'_, Result<Subscription>>;
    fn publish(&self, event: Event) -> BoxFuture<'_, Result<()>>;
}

struct Reader {
    interest: Interest,
    queue: mpsc::Sender<Arc<Event>>,
}
#[derive(Default)]
struct Readers {
    next: u64,
    entries: BTreeMap<u64, Reader>,
}
#[derive(Default, Clone)]
pub struct Memory(Arc<Mutex<Readers>>);
struct Registration {
    readers: Weak<Mutex<Readers>>,
    id: u64,
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(readers) = self.readers.upgrade() {
            readers.lock().unwrap().entries.remove(&self.id);
        }
    }
}
impl Broker for Memory {
    fn subscribe(&self, interest: Interest) -> BoxFuture<'_, Result<Subscription>> {
        Box::pin(async move {
            let (tx, mut rx) = mpsc::channel(32);
            let mut readers = self.0.lock().unwrap();
            let id = readers.next;
            readers.next += 1;
            readers.entries.insert(
                id,
                Reader {
                    interest,
                    queue: tx,
                },
            );
            let registration = Registration {
                readers: Arc::downgrade(&self.0),
                id,
            };
            Ok(async_stream::try_stream! {
                let _registration=registration;
                while let Some(event)=rx.next().await {yield event}
                Err("watch subscription interrupted; reconnect".to_string())?;
            }
            .boxed())
        })
    }
    fn publish(&self, event: Event) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let event = Arc::new(event);
            self.0.lock().unwrap().entries.retain(|_, r| {
                !r.interest.accepts(&event) || r.queue.try_send(event.clone()).is_ok()
            });
            Ok(())
        })
    }
}
