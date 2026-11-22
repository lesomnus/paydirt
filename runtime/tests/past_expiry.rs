#![cfg(feature = "sqlite")]
use base64::{Engine, engine::general_purpose::STANDARD};
use paydirt_store::{
    Call, Frame, Inputs, Packet, Scope, Store,
    past_expiry::{KeepFact, Window},
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};
fn run(url: &str, project: &Path) {
    let read =
        |file: &str| std::fs::read_to_string(project.join("generated/storage").join(file)).unwrap();
    let clock = Arc::new(AtomicI64::new(1000));
    let now = clock.clone();
    let mut db = Store::open(url, &read("model.json"), &read("policies.json"))
        .unwrap()
        .with_inputs(Inputs::new(
            move || chrono::DateTime::from_timestamp(now.load(Ordering::SeqCst), 0).unwrap(),
            paydirt::mint,
        ));
    let mut tenants = Vec::new();
    for alias in ["history", "other-history"] {
        let tenant = paydirt::mint(1);
        let t = STANDARD.encode(tenant);
        db.call(
            &Scope::Internal,
            Call {
                entity: "app.Tenant".into(),
                method: "Add".into(),
                request: json!({"id":t,"alias":alias}),
            },
        )
        .unwrap();
        let robot = db
            .call(
                &Scope::Tenant(tenant.to_vec()),
                Call {
                    entity: "app.Robot".into(),
                    method: "Add".into(),
                    request: json!({"alias":"machine","tenant":{"id":t}}),
                },
            )
            .unwrap();
        let frame: Frame = serde_json::from_value(
            json!({"tenant":tenant,"actor":paydirt::mint(2),"action":"app.Journal"}),
        )
        .unwrap();
        let mut prev = Value::Null;
        for (i, label) in ["first", "baseline", "later"].into_iter().enumerate() {
            let recorded = 1000 + 100 * i as i64;
            clock.store(recorded, Ordering::SeqCst);
            let date = chrono::DateTime::from_timestamp(recorded, 0)
                .unwrap()
                .to_rfc3339();
            let after = json!({"id":robot["id"],"alias":label});
            db.handle(Packet::Begin {
                id: "op".into(),
                frame: frame.clone(),
            })
            .unwrap();
            db.handle(Packet::Call{id:"op".into(),call:Call{entity:"app.Robot".into(),method:"$event".into(),request:json!({"kind":"robot.move","value":{"@type":"type.googleapis.com/app.RobotMoved","robot":robot["id"]},"before":prev,"after":after,"dateStart":date})}}).unwrap();
            db.handle(Packet::Finish {
                id: "op".into(),
                commit: true,
            })
            .unwrap();
            prev = after;
        }
        tenants.push((tenant, robot));
    }
    clock.store(1300, Ordering::SeqCst);
    let tenant = tenants[0].0;
    let w = Window {
        keep_ms: 150000,
        ..Default::default()
    };
    let state = |db: &mut Store, t: &([u8; 16], Value), at: i64| {
        let stamp = chrono::DateTime::from_timestamp(at, 0)
            .unwrap()
            .to_rfc3339();
        db.call(
            &Scope::Tenant(t.0.to_vec()),
            Call {
                entity: "app.Robot".into(),
                method: "$state".into(),
                request: json!({"ref":{"id":t.1["id"]},"at":stamp,"known":stamp}),
            },
        )
        .unwrap()["facts"][0]["value"]
            .clone()
    };
    assert_eq!(state(&mut db, &tenants[0], 1155), "\"baseline\"");
    let hold = Window {
        holds: vec!["review".into()],
        ..w.clone()
    };
    assert!(
        db.expire_past(tenant, &hold, false)
            .unwrap()
            .facts
            .is_empty()
    );
    let keep = Window {
        keep_facts: vec![KeepFact {
            domain: 7,
            field: 4,
            key: "".into(),
            values: vec!["\"first\"".into()],
        }],
        ..w.clone()
    };
    assert!(
        db.expire_past(tenant, &keep, true)
            .unwrap()
            .facts
            .is_empty()
    );
    let preview = db.expire_past(tenant, &w, true).unwrap();
    assert_eq!(preview.facts.len(), 1);
    assert_eq!(preview.events.len(), 1);
    let actual = db.expire_past(tenant, &w, false).unwrap();
    assert_eq!(preview.facts, actual.facts);
    assert_eq!(preview.events, actual.events);
    assert_eq!(state(&mut db, &tenants[0], 1155), "\"baseline\"");
    assert_eq!(state(&mut db, &tenants[0], 1250), "\"later\"");
    assert_eq!(state(&mut db, &tenants[1], 1050), "\"first\"");
    assert!(db.expire_past(tenant, &w, false).unwrap().events.is_empty());
}
#[test]
fn generated_history_window() {
    let Some(project) = std::env::var_os("CELL_PROJECT") else {
        return;
    };
    run(":memory:", Path::new(&project));
    #[cfg(feature = "postgres")]
    if let Ok(url) = std::env::var("DATABASE_URL") {
        use diesel::{Connection, connection::SimpleConnection};
        let mut db = diesel::PgConnection::establish(&url).unwrap();
        let schema = format!("past_{}", hex::encode(paydirt::mint(0)));
        db.batch_execute(&format!("CREATE SCHEMA {schema}"))
            .unwrap();
        let scoped = format!(
            "{url}{}options=-csearch_path%3D{schema}",
            if url.contains('?') { "&" } else { "?" }
        );
        let result = std::panic::catch_unwind(|| run(&scoped, Path::new(&project)));
        db.batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
            .unwrap();
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }
}
