#![cfg(any(feature = "sqlite", feature = "postgres"))]
use base64::{Engine, engine::general_purpose::STANDARD};
use paydirt_store::{
    Call, Fault, Scope, Store,
    admission::{Decision, Options},
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
const MODEL: &str = include_str!("../../generated/storage/model.json");
const POLICY: &str = include_str!("../../generated/storage/policies.json");
const SCHEMA: &str = include_str!("../../generated/storage/admission.json");
fn options() -> Options {
    Options {
        entities: vec!["app.Cell".into()],
        ..Default::default()
    }
}
fn call(method: &str, request: Value) -> Call {
    Call {
        entity: "app.Cell".into(),
        method: method.into(),
        request,
    }
}
fn exercise(url: &str) {
    let seen: Arc<Mutex<Vec<Decision>>> = Default::default();
    let copy = seen.clone();
    let mut db = Store::open(url, MODEL, POLICY)
        .unwrap()
        .with_admission(SCHEMA, options(), move |d| {
            let deny = d.value.as_ref().is_some_and(|v| v["alias"] == "reject");
            copy.lock().unwrap().push(d);
            if deny {
                Err(Fault::denied("rejected"))
            } else {
                Ok(())
            }
        })
        .unwrap();
    let tenant = paydirt::mint(1);
    let key = STANDARD.encode(tenant);
    db.seed(vec![(
        "app.Tenant".into(),
        json!({"id":key,"alias":format!("test-{}",hex::encode(tenant))}),
    )])
    .unwrap();
    let scope = Scope::Tenant(tenant.to_vec());
    let make = |alias: &str| json!({"tenant":{"id":key},"alias":alias});
    let a = db.call(&scope, call("Add", make("sample"))).unwrap();
    let key = json!({"id":a["id"]});
    db.call(&scope, call("Patch", json!({"ref":key,"alias":"patched"})))
        .unwrap();
    let batch = json!({"items":[make("first"),make("reject")]});
    assert_eq!(
        db.call(&scope, call("AddMany", batch.clone()))
            .unwrap_err()
            .code,
        "permission_denied"
    );
    db.call(&scope, call("Erase", key.clone())).unwrap();
    db.call(&scope, call("Erase", key.clone())).unwrap();
    let calls = seen.lock().unwrap();
    assert_eq!(calls.len(), 5);
    assert!(calls[0].previous.is_none());
    assert_eq!(calls[1].previous.as_ref().unwrap()["alias"], "sample");
    assert_eq!(calls[1].value.as_ref().unwrap()["alias"], "patched");
    assert_eq!(calls[2].by, "/app.CellService/Add");
    assert_eq!(calls[3].request, batch);
    assert_eq!(calls[4].object, a["id"]);
    assert!(calls[4].value.is_none());
    let failed_key = json!({"id":calls[2].object});
    drop(calls);
    assert_eq!(
        db.call(&scope, call("Get", json!({"ref":failed_key})))
            .unwrap_err()
            .code,
        "not_found"
    );
}
#[cfg(feature = "sqlite")]
#[test]
fn sqlite() {
    exercise(":memory:")
}
#[cfg(feature = "postgres")]
#[test]
fn postgres() {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        exercise(&url)
    }
}
#[cfg(feature = "sqlite")]
#[test]
fn invalid_selectors_and_timed_out_rules() {
    let install = |name: &str| {
        Store::open(":memory:", MODEL, POLICY)
            .unwrap()
            .with_admission(
                SCHEMA,
                Options {
                    entities: vec![name.into()],
                    ..Default::default()
                },
                |_| Ok(()),
            )
    };
    assert!(install("app.NoSuch").is_err());
    assert!(install("app.Audit").is_err());
    let mut db = Store::open(":memory:", MODEL, POLICY)
        .unwrap()
        .with_admission(
            SCHEMA,
            Options {
                entities: vec!["app.Tenant".into()],
                timeout: Duration::from_millis(20),
                ..Default::default()
            },
            |_| {
                std::thread::sleep(Duration::from_millis(100));
                Ok(())
            },
        )
        .unwrap();
    let key = STANDARD.encode(paydirt::mint(1));
    assert_eq!(
        db.operator(Call {
            entity: "app.Tenant".into(),
            method: "Add".into(),
            request: json!({"alias":"slow","id":key})
        })
        .unwrap_err()
        .code,
        "deadline_exceeded"
    );
    assert_eq!(
        db.operator(Call {
            entity: "app.Tenant".into(),
            method: "Get".into(),
            request: json!({"ref":{"id":key}})
        })
        .unwrap_err()
        .code,
        "not_found"
    );
}
