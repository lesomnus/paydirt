//! Host-side publishing of committed writes. Acknowledgement is a separate
//! transaction so a crash between publish and deletion leaves the row retryable.
use crate::{
    Fault, Result, Scope, Store,
    driver::{Bind, Query},
    value,
};
use paydirt::watch::{Broker, Change, Event};
use paydirt_model::quote;
#[derive(Debug)]
pub struct QueuedEvent {
    pub id: Vec<u8>,
    pub event: Event,
}
impl Store {
    pub fn queued_events(&mut self, limit: u32) -> Result<Vec<QueuedEvent>> {
        self.transaction(&Scope::Internal, |tx| {
            let e = tx.owned_entity("OWN_OUTBOX")?;
            let key = e.key().map_err(Fault::internal)?;
            let query = Query::new(format!(
                "SELECT {} AS value FROM {} r ORDER BY r.{} LIMIT {}",
                tx.project_sql(e)?,
                quote(&e.table()),
                quote(&key.column()),
                limit
            ));
            let rows = tx.db.rows(query)?;
            rows.iter()
                .map(|raw| {
                    let e = tx.owned_entity("OWN_OUTBOX")?;
                    let row = tx.decode_row(e, raw)?;
                    let id = value::bytes(&row["id"])?;
                    let uuid = |field: &str| {
                        value::bytes(&row[field])
                            .ok()
                            .and_then(|v| paydirt::format_id(&v))
                            .unwrap_or_default()
                    };
                    let routing = row
                        .get("routes")
                        .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                        .map(value::bytes)
                        .transpose()?;
                    let routes = routing
                        .map(|v| {
                            serde_json::from_slice(&v).map_err(|e| Fault::internal(e.to_string()))
                        })
                        .transpose()?
                        .flatten();
                    let method = row["method"].as_str().unwrap_or_default().to_owned();
                    let change = Change {
                        key: uuid("objectId"),
                        by: row["by"].as_str().unwrap_or_default().into(),
                        method: method.clone(),
                        routes,
                        signal: row["signal"].as_str().unwrap_or_default().into(),
                        resource: row["event"].as_str().unwrap_or_default().into(),
                    };
                    Ok(QueuedEvent {
                        id,
                        event: Event {
                            actor: uuid("actorId"),
                            tenant: uuid("tenantId"),
                            method,
                            changes: vec![change],
                        },
                    })
                })
                .collect()
        })
    }
    pub fn acknowledge_event(&mut self, id: &[u8]) -> Result<()> {
        self.transaction(&Scope::Internal, |tx| {
            let e = tx.owned_entity("OWN_OUTBOX")?;
            let table = quote(&e.table());
            let key = quote(&e.key().map_err(Fault::internal)?.column());
            let mut q = Query::default();
            let param = q.param(Bind::Bytes(id.to_vec()), tx.db.dialect());
            q.sql = format!("DELETE FROM {table} WHERE {key}={param}");
            tx.db.exec(q)?;
            Ok(())
        })
    }
    pub async fn drain_once(&mut self, broker: &dyn Broker, limit: u32) -> Result<usize> {
        let batch = self.queued_events(limit)?;
        let size = batch.len();
        for record in batch {
            broker
                .publish(record.event)
                .await
                .map_err(Fault::internal)?;
            self.acknowledge_event(&record.id)?;
        }
        Ok(size)
    }
}
