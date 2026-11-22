#![cfg(feature = "sqlite")]
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::future::BoxFuture;
use paydirt::watch::{Broker, Event, Interest, Subscription};
use paydirt_store::{Call, Frame, Packet, Scope, Store};
use serde_json::json;
use std::sync::Mutex;

#[derive(Default)]
struct Publisher {
    fail: bool,
    events: Mutex<Vec<Event>>,
}
impl Broker for Publisher {
    fn publish(&self, event: Event) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            if self.fail {
                return Err("broker temporarily offline".into());
            }
            self.events.lock().unwrap().push(event);
            Ok(())
        })
    }
    fn subscribe(&self, _: Interest) -> BoxFuture<'_, Result<Subscription, String>> {
        Box::pin(async { Err("publisher only".into()) })
    }
}
#[tokio::test]
async fn publication_is_confirmed_before_deletion() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db");
    let open = || {
        Store::open(
            path.to_str().unwrap(),
            include_str!("../../generated/storage/model.json"),
            include_str!("../../generated/storage/policies.json"),
        )
        .unwrap()
    };
    let mut store = open();
    let tenant = paydirt::mint(1);
    let id = STANDARD.encode(tenant);
    store
        .call(
            &Scope::Internal,
            Call {
                entity: "app.Tenant".into(),
                method: "Add".into(),
                request: json!({"id":id,"alias":"example"}),
            },
        )
        .unwrap();
    let idtx = "request";
    let add = |store: &mut Store, commit: bool, alias: &str| {
        let frame = Frame {
            address: None,
            row: None,
            caller_row: None,
            counterpart: None,
            tenant: tenant.to_vec(),
            actor: Some(paydirt::mint(2).to_vec()),
            account: None,
            topics: Default::default(),
            tenants: None,
            sets: None,
            caller: None,
            caller_tenant: None,
            presence: None,
            trace: vec![0; 16],
            action: "/app.CellService/Add".into(),
        };
        store
            .handle(Packet::Begin {
                id: idtx.into(),
                frame,
            })
            .unwrap();
        store
            .handle(Packet::Call {
                id: idtx.into(),
                call: Call {
                    entity: "app.Cell".into(),
                    method: "Add".into(),
                    request: json!({"tenant":{"id":id},"alias":alias}),
                },
            })
            .unwrap();
        store
            .handle(Packet::Finish {
                id: idtx.into(),
                commit,
            })
            .unwrap()
    };
    assert!(add(&mut store, false, "nothing").is_null());
    assert!(store.queued_events(10).unwrap().is_empty());
    let committed: Event = serde_json::from_value(add(&mut store, true, "one")).unwrap();
    assert_eq!(committed.changes.len(), 1);
    assert!(
        store
            .drain_once(
                &Publisher {
                    fail: true,
                    ..Default::default()
                },
                10
            )
            .await
            .is_err()
    );
    let broker = Publisher::default();
    let pending = store.queued_events(10).unwrap();
    assert_eq!(pending.len(), 1);
    broker.publish(pending[0].event.clone()).await.unwrap();
    // Simulate the process stopping after the broker acknowledged the event.
    drop(store);
    let mut store = open();
    assert_eq!(store.drain_once(&broker, 10).await.unwrap(), 1);
    assert_eq!(store.drain_once(&broker, 10).await.unwrap(), 0);
    let events = broker.events.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0], events[1]);
    assert_eq!(events[0].changes[0].key, committed.changes[0].key);
    assert_eq!(events[0].method, committed.method);
}
