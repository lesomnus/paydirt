//! Core NATS adapter. Each native interest is installed before a Watch becomes ready.
use super::{Broker, Change, Event, Interest, Result, Subscription, route_key, service_of, wire};
use futures_util::{StreamExt, future::BoxFuture, stream::SelectAll};
use std::sync::Arc;
use uuid::Uuid;
pub struct Nats {
    client: async_nats::Client,
    prefix: String,
    generation: tokio::sync::watch::Receiver<u64>,
}
fn token(value: &str) -> String {
    route_key(&["payday.watch.nats", value])
}
fn tokens(values: &[String]) -> Vec<String> {
    if values.is_empty() {
        return vec!["*".into()];
    }
    let mut result = vec!["u".into()];
    result.extend(values.iter().map(|v| token(v)));
    result.sort();
    result.dedup();
    result
}
impl Nats {
    pub async fn connect(url: &str, namespace: &str) -> Result<Self> {
        if namespace.is_empty() || url.is_empty() {
            return Err("NATS needs a URL and deployment namespace".into());
        }
        let (changed, generation) = tokio::sync::watch::channel(0u64);
        let client = async_nats::ConnectOptions::new()
            .event_callback(move |event| {
                let changed = changed.clone();
                async move {
                    if !matches!(
                        event,
                        async_nats::Event::Connected | async_nats::Event::LameDuckMode
                    ) {
                        changed.send_modify(|n| *n = n.wrapping_add(1));
                    }
                }
            })
            .connect(url)
            .await
            .map_err(|e| e.to_string())?;
        if client.server_info().max_payload < 1048 {
            return Err("NATS max_payload below 1048 bytes".into());
        }
        Ok(Self {
            client,
            prefix: format!("payday.w1.{}", token(namespace)),
            generation,
        })
    }
    fn patterns(&self, i: &Interest) -> Result<Vec<String>> {
        let services = if i.service.is_empty() {
            vec!["*".into()]
        } else {
            vec![token(&i.service), "u".into()]
        };
        let keys = tokens(&i.keys);
        let routes = tokens(&i.routes);
        if keys.len() > 8192 / services.len() / routes.len() {
            return Err("NATS interest exceeds 8192 bindings".into());
        }
        let mut subjects = Vec::new();
        for s in &services {
            for k in &keys {
                for r in &routes {
                    subjects.push(format!("{}.{s}.{k}.{r}", self.prefix));
                }
            }
        }
        Ok(subjects)
    }
    fn subjects(&self, c: &Change) -> Vec<String> {
        let service = if c.by.is_empty() {
            "u".into()
        } else {
            token(service_of(&c.by))
        };
        let key = if c.key.is_empty() || c.key == "00000000-0000-0000-0000-000000000000" {
            "u".into()
        } else {
            token(&c.key)
        };
        let mut routes = match &c.routes {
            None => vec!["u".into()],
            Some(r) if r.is_empty() => vec!["n".into()],
            Some(r) => r.iter().map(|s| token(s)).collect(),
        };
        routes.sort();
        routes.dedup();
        routes
            .into_iter()
            .map(|r| format!("{}.{service}.{key}.{r}", self.prefix))
            .collect()
    }
}
impl Broker for Nats {
    fn subscribe(&self, interest: Interest) -> BoxFuture<'_, Result<Subscription>> {
        Box::pin(async move {
            let mut generation = self.generation.clone();
            generation.borrow_and_update();
            let mut streams = SelectAll::new();
            for s in self.patterns(&interest)? {
                streams.push(self.client.subscribe(s).await.map_err(|e| e.to_string())?)
            }
            self.client.flush().await.map_err(|e| e.to_string())?;
            if generation.has_changed().unwrap_or(true) {
                return Err("NATS registration interrupted".into());
            }
            Ok(async_stream::try_stream!{
                let mut parts=wire::Receiver::default();
                loop{
                    let msg=tokio::select!{
                        biased;
                        _=generation.changed()=>Err("NATS watch disconnected; reconnect"),
                        _=wire::expiration(parts.deadline())=>Err("incomplete NATS publication"),
                        msg=streams.next()=>msg.ok_or("NATS subscription ended"),
                    }?;
                    let data=msg.payload;
                    if data.len()<=24{Err("short NATS fragment")?}
                    let id=Uuid::from_bytes(data[0..16].try_into().unwrap());
                    let index=u32::from_be_bytes(data[16..20].try_into().unwrap()) as usize;
                    let count=u32::from_be_bytes(data[20..24].try_into().unwrap()) as usize;
                    if let Some(e)=parts.accept(id,index,count,&data[24..])?.filter(|e|interest.accepts(e)){yield Arc::new(e)}
                }
            }.boxed())
        })
    }
    fn publish(&self, event: Event) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let mut generation = self.generation.clone();
            generation.borrow_and_update();
            let chunk = (self.client.server_info().max_payload - 24).min(64 << 10);
            let mut budget = 0;
            for c in event.changes.iter() {
                let single = Event {
                    actor: event.actor.clone(),
                    tenant: event.tenant.clone(),
                    method: event.method.clone(),
                    changes: vec![c.clone()],
                };
                let data = wire::encode(&single)?;
                let count = data.len().div_ceil(chunk) as u32;
                let subjects = self.subjects(c);
                budget += (data.len() + count as usize * 24) * subjects.len();
                if budget > wire::MAX {
                    return Err("NATS publication exceeds queue budget".into());
                }
                let id = Uuid::now_v7();
                for (i, part) in data.chunks(chunk).enumerate() {
                    let mut packet = Vec::with_capacity(24 + part.len());
                    packet.extend(id.as_bytes());
                    packet.extend((i as u32).to_be_bytes());
                    packet.extend(count.to_be_bytes());
                    packet.extend(part);
                    let payload = bytes::Bytes::from(packet);
                    for subject in &subjects {
                        self.client
                            .publish(subject.clone(), payload.clone())
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
            self.client.flush().await.map_err(|e| e.to_string())?;
            if generation.has_changed().unwrap_or(true) {
                return Err("NATS publication interrupted".into());
            }
            Ok(())
        })
    }
}
