#![cfg(feature = "watch-postgres")]
use paydirt::spin::{Lease, Placement, Stop, Workers};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
#[tokio::test]
async fn failure_stops_the_server_and_waits_for_clean_up() {
    let mut tasks = Workers::default();
    let observed = Arc::new(AtomicUsize::new(0));
    let marker = observed.clone();
    tasks
        .add("observer", Placement::Everywhere, move |stop| {
            let m = marker.clone();
            async move {
                stop.cancelled().await;
                m.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .unwrap();
    tasks
        .add("failing", Placement::Everywhere, |_| async {
            Err("broken".into())
        })
        .unwrap();
    let res = tasks
        .run(Lease::Local, std::future::pending(), |stop| async move {
            stop.cancelled().await;
            Ok(())
        })
        .await;
    assert_eq!(res.unwrap_err(), "failing: broken");
    assert_eq!(observed.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn postgres_singleton_hands_off_its_dedicated_session() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let name = format!("test.{}", uuid::Uuid::now_v7());
    let (tx, mut rx) = mpsc::channel(4);
    let active = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    let mut stops = Vec::new();
    for id in 0..2 {
        let mut group = Workers::default();
        let active = active.clone();
        let tx = tx.clone();
        group
            .add(name.clone(), Placement::Singleton, move |stop: Stop| {
                let tx = tx.clone();
                let active = active.clone();
                async move {
                    assert_eq!(active.fetch_add(1, Ordering::SeqCst), 0);
                    tx.send(id).await.unwrap();
                    stop.cancelled().await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .unwrap();
        let (end, wait) = oneshot::channel();
        stops.push(Some(end));
        let lease = Lease::Postgres {
            dsn: url.clone(),
            retry: Duration::from_millis(30),
        };
        handles.push(tokio::spawn(group.run(
            lease,
            async move {
                let _ = wait.await;
            },
            |stop| async move {
                stop.cancelled().await;
                Ok(())
            },
        )));
    }
    let leader = tokio::time::timeout(Duration::from_secs(4), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), rx.recv())
            .await
            .is_err()
    );
    stops[leader].take().unwrap().send(()).unwrap();
    let second = tokio::time::timeout(Duration::from_secs(4), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(leader, second);
    stops[second].take().unwrap().send(()).unwrap();
    for h in handles {
        h.await.unwrap().unwrap();
    }
}
