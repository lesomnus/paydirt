use futures_util::StreamExt;
use paydirt::watch::{Broker, Change, Event, Interest, Memory};
use std::time::Duration;
fn event(service: &str, key: &str) -> Event {
    let m = format!("/{service}/Patch");
    Event {
        method: m.clone(),
        changes: vec![Change {
            by: m,
            key: key.into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}
async fn replicas(reader: &dyn Broker, writer: &dyn Broker) {
    let id = uuid::Uuid::now_v7().to_string();
    let watch = Interest {
        service: "/app.RobotService/".into(),
        keys: vec![id.clone()],
        ..Default::default()
    };
    let mut sub = reader.subscribe(watch).await.unwrap();
    writer
        .publish(event("app.OtherService", &id))
        .await
        .unwrap();
    writer
        .publish(event("app.RobotService", &uuid::Uuid::now_v7().to_string()))
        .await
        .unwrap();
    let wanted = event("app.RobotService", &id);
    writer.publish(wanted.clone()).await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(*received, wanted);
}
#[tokio::test]
async fn memory_drops_slow_subscribers_only() {
    let b = Memory::default();
    replicas(&b, &b).await;
    let mut slow = b.subscribe(Interest::default()).await.unwrap();
    let mut fast = b.subscribe(Interest::default()).await.unwrap();
    let event = event("app.RobotService", &uuid::Uuid::now_v7().to_string());
    for _ in 0..64 {
        b.publish(event.clone()).await.unwrap();
        assert!(fast.next().await.unwrap().is_ok())
    }
    while let Some(result) = slow.next().await {
        if result.is_err() {
            return;
        }
    }
    panic!("expected terminal stream error");
}
#[cfg(feature = "watch-postgres")]
#[tokio::test]
async fn pg_uses_independent_connections() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let reader = paydirt::watch::postgres::Postgres::new(&url);
    let writer = paydirt::watch::postgres::Postgres::new(&url);
    replicas(&reader, &writer).await;
    let mut sub = reader.subscribe(Interest::default()).await.unwrap();
    let mut big = event("app.Big", &uuid::Uuid::now_v7().to_string());
    big.changes[0].resource = "A".repeat(32_000);
    writer.publish(big.clone()).await.unwrap();
    assert_eq!(
        *tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        big
    );
}
#[cfg(feature = "watch-nats")]
#[tokio::test]
async fn nats_filters_at_the_server_and_isolates_namespaces() {
    use paydirt::watch::nats::Nats;
    let Ok(url) = std::env::var("NATS_URL") else {
        return;
    };
    let ns = uuid::Uuid::now_v7().to_string();
    let reader = Nats::connect(&url, &ns).await.unwrap();
    let writer = Nats::connect(&url, &ns).await.unwrap();
    let other = Nats::connect(&url, &format!("{ns}-other")).await.unwrap();
    let mut isolated = other.subscribe(Interest::default()).await.unwrap();
    replicas(&reader, &writer).await;
    let mut sub = reader
        .subscribe(Interest {
            routes: vec!["public".into(), "shared".into()],
            ..Default::default()
        })
        .await
        .unwrap();
    let mut e = event("app.Big", &uuid::Uuid::now_v7().to_string());
    e.changes[0].routes = Some(vec!["private".into()]);
    writer.publish(e.clone()).await.unwrap();
    e.changes[0].routes = Some(vec!["public".into(), "shared".into()]);
    e.changes[0].resource = "A".repeat(140_000);
    writer.publish(e.clone()).await.unwrap();
    assert_eq!(
        *tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        e
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), sub.next())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), isolated.next())
            .await
            .is_err()
    );
}

#[cfg(feature = "watch-nats")]
#[tokio::test]
async fn nats_disconnect_ends_existing_stream_and_reconnects() {
    use paydirt::watch::nats::Nats;
    use tokio::{
        io::copy_bidirectional,
        net::{TcpListener, TcpStream},
    };
    let Ok(url) = std::env::var("NATS_URL") else {
        return;
    };
    let origin = url.trim_start_matches("nats://").to_owned();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxied = format!("nats://{}", listener.local_addr().unwrap());
    let (drop_clients, rx) = tokio::sync::watch::channel(0u8);
    let proxy = tokio::spawn(async move {
        while let Ok((mut downstream, _)) = listener.accept().await {
            let mut signal = rx.clone();
            signal.borrow_and_update();
            let mut upstream = TcpStream::connect(&origin).await.unwrap();
            tokio::spawn(async move {
                tokio::select! {_=signal.changed()=>{},_=copy_bidirectional(&mut downstream,&mut upstream)=>{}}
            });
        }
    });
    let ns = uuid::Uuid::now_v7().to_string();
    let reader = Nats::connect(&proxied, &ns).await.unwrap();
    let writer = Nats::connect(&url, &ns).await.unwrap();
    let mut old = reader.subscribe(Interest::default()).await.unwrap();
    drop_clients.send(1).unwrap();
    let last = tokio::time::timeout(Duration::from_secs(5), old.next())
        .await
        .unwrap()
        .unwrap();
    assert!(last.is_err());
    tokio::time::timeout(Duration::from_secs(8), replicas(&reader, &writer))
        .await
        .unwrap();
    proxy.abort();
}

#[tokio::test]
async fn registered_brokers_report_supported_names_and_refuse_an_empty_factory() {
    use paydirt::watch::{BrokerSettings, broker_names, register_broker, registered_broker};
    use std::sync::Arc;
    let settings = BrokerSettings {
        database: "tenant".into(),
        dsn: String::new(),
        namespace: "application".into(),
    };
    let err = registered_broker("missing-fixture", settings.clone())
        .await
        .err()
        .unwrap();
    for word in ["missing-fixture", "memory", "register_broker"] {
        assert!(err.contains(word));
    }
    register_broker("fixture-echo", |s| {
        Box::pin(async move {
            assert_eq!(s.namespace, "application");
            Ok(Some(Arc::new(Memory::default()) as Arc<dyn Broker>))
        })
    })
    .unwrap();
    let broker = registered_broker("fixture-echo", settings.clone())
        .await
        .unwrap();
    let mut sub = broker.subscribe(Interest::default()).await.unwrap();
    let e = event("test", &uuid::Uuid::now_v7().to_string());
    broker.publish(e.clone()).await.unwrap();
    assert_eq!(*sub.next().await.unwrap().unwrap(), e);
    assert!(
        ["memory", "none", "fixture-echo"]
            .iter()
            .all(|s| broker_names().contains(&s.to_string()))
    );
    register_broker("fixture-empty", |_| Box::pin(async { Ok(None) })).unwrap();
    let err = registered_broker("fixture-empty", settings)
        .await
        .err()
        .unwrap();
    assert!(err.contains("built no broker") && err.contains("none"));
}
