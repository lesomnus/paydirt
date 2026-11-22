//! PostgreSQL adapter for the Go payday_watch channel and wire envelope.
use super::{Broker, Event, Interest, Result, Subscription, wire};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{
    StreamExt,
    future::{BoxFuture, poll_fn},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio_postgres::{AsyncMessage, NoTls};
use uuid::Uuid;
const CHUNK: usize = 4500;
#[derive(Serialize, Deserialize)]
struct Fragment {
    f: String,
    i: usize,
    n: usize,
    d: String,
}
pub struct Postgres {
    url: String,
}
impl Postgres {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}
impl Broker for Postgres {
    fn subscribe(&self, interest: Interest) -> BoxFuture<'_, Result<Subscription>> {
        Box::pin(async move {
            let (client, mut connection) = tokio_postgres::connect(&self.url, NoTls)
                .await
                .map_err(|e| e.to_string())?;
            let (tx, mut rx) = tokio::sync::mpsc::channel(64);
            tokio::spawn(async move {
                while let Some(Ok(message)) = poll_fn(|ctx| connection.poll_message(ctx)).await {
                    if let AsyncMessage::Notification(n) = message
                        && tx.try_send(n.payload().to_owned()).is_err()
                    {
                        break;
                    }
                }
            });
            client
                .batch_execute("LISTEN payday_watch")
                .await
                .map_err(|e| e.to_string())?;
            Ok(async_stream::try_stream! {
                let _connection=client;
                let mut parts=wire::Receiver::default();
                loop{
                    let payload=tokio::select!{
                        p=rx.recv()=>p.ok_or("PostgreSQL watch connection interrupted"),
                        _=wire::expiration(parts.deadline())=>Err("incomplete PostgreSQL watch publication"),
                    }?;
                    let packet:serde_json::Value=serde_json::from_str(&payload).map_err(|e|e.to_string())?;
                    let event=if packet.get("f").is_some(){
                        let f:Fragment=serde_json::from_value(packet).map_err(|e|e.to_string())?;
                        let bytes=STANDARD.decode(&f.d).map_err(|e|e.to_string())?;
                        if bytes.len()>CHUNK{Err("oversized PostgreSQL watch fragment")?;}
                        parts.accept(Uuid::parse_str(&f.f).map_err(|e|e.to_string())?,f.i,f.n,&bytes)?
                    }else{Some(wire::decode(payload.as_bytes())?)};
                    if let Some(event)=event.filter(|e|interest.accepts(e)){yield Arc::new(event)}
                }
            }.boxed())
        })
    }
    fn publish(&self, event: Event) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            if event.changes.is_empty() {
                return Ok(());
            }
            let data = wire::encode(&event)?;
            let (mut client, connection) = tokio_postgres::connect(&self.url, NoTls)
                .await
                .map_err(|e| e.to_string())?;
            tokio::spawn(connection);
            let tx = client.transaction().await.map_err(|e| e.to_string())?;
            let id = Uuid::now_v7().to_string();
            let packets = if data.len() < 7900 {
                vec![String::from_utf8(data).map_err(|e| e.to_string())?]
            } else {
                data.chunks(CHUNK)
                    .enumerate()
                    .map(|(i, part)| {
                        serde_json::to_string(&Fragment {
                            f: id.clone(),
                            i,
                            n: data.len().div_ceil(CHUNK),
                            d: STANDARD.encode(part),
                        })
                        .map_err(|e| e.to_string())
                    })
                    .collect::<Result<Vec<_>>>()?
            };
            for packet in packets {
                tx.execute("SELECT pg_notify('payday_watch',$1)", &[&packet])
                    .await
                    .map_err(|e| e.to_string())?;
            }
            tx.commit().await.map_err(|e| e.to_string())
        })
    }
}
