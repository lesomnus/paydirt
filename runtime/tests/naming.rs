#![cfg(any(feature = "sqlite", feature = "postgres"))]
use paydirt_store::{Call, Scope, Store, naming};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn call(
    store: &mut Store,
    entity: &str,
    method: &str,
    request: Value,
) -> paydirt_store::Result<Value> {
    store.call(
        &Scope::Internal,
        Call {
            entity: format!("app.{entity}"),
            method: method.into(),
            request,
        },
    )
}
fn exercise(url: &str) {
    let mut store = Store::open(
        url,
        include_str!("../../generated/storage/model.json"),
        include_str!("../../generated/storage/policies.json"),
    )
    .unwrap();
    let tenant = call(&mut store, "Tenant", "Add", json!({})).unwrap();
    let tenant_ref = json!({"id":tenant["id"]});
    let row = call(&mut store, "Robot", "Add", json!({"tenant":tenant_ref})).unwrap();
    let name = row["alias"].as_str().unwrap();
    assert_eq!(name.len(), 7);
    assert!(paydirt::rbac::alias(name));
    let slug = json!({"slug":{"alias":name,"tenant":tenant_ref}});
    assert_eq!(
        call(&mut store, "Robot", "Get", json!({"ref":slug})).unwrap()["id"],
        row["id"]
    );
    let row = call(
        &mut store,
        "Robot",
        "Add",
        json!({"tenant":tenant_ref,"alias":"  Taken  "}),
    )
    .unwrap();
    assert_eq!(row["alias"], "taken");

    let attempts = Arc::new(AtomicUsize::new(0));
    let count = attempts.clone();
    let mut store = store.with_namer(move |r: naming::NameRequest<'_>| {
        if r.entity == "app.Tenant" {
            return naming::required(r);
        }
        if !r.given.is_empty() {
            return naming::names(r);
        }
        assert_eq!(r.request["category"], "fixture");
        Ok(if count.fetch_add(1, Ordering::SeqCst) == 0 {
            "taken"
        } else {
            "another"
        }
        .into())
    });
    assert_eq!(
        call(&mut store, "Tenant", "Add", json!({}))
            .unwrap_err()
            .field
            .as_deref(),
        Some("alias")
    );
    let second = call(
        &mut store,
        "Robot",
        "Add",
        json!({"tenant":tenant_ref,"category":"fixture"}),
    )
    .unwrap();
    assert_eq!(second["alias"], "another");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let duplicate = call(
        &mut store,
        "Robot",
        "Add",
        json!({"tenant":tenant_ref,"alias":"Taken"}),
    )
    .unwrap_err();
    assert_eq!(duplicate.code, "already_exists");
    let id = row["id"].clone();
    let mut store = store.with_namer(|_: naming::NameRequest<'_>| Ok("otherwise-free".into()));
    assert_eq!(
        call(
            &mut store,
            "Robot",
            "Add",
            json!({"tenant":tenant_ref,"id":id})
        )
        .unwrap_err()
        .code,
        "already_exists"
    );
    let updated = call(
        &mut store,
        "Robot",
        "Patch",
        json!({"ref":{"id":id},"dateUpdatedForce":true}),
    )
    .unwrap();
    assert_eq!(updated["alias"], "taken");
    assert_eq!(
        call(
            &mut store,
            "Robot",
            "Patch",
            json!({"ref":{"id":id},"dateUpdatedForce":true,"alias":""})
        )
        .unwrap_err()
        .field
        .as_deref(),
        Some("alias")
    );
}

#[cfg(feature = "sqlite")]
#[test]
fn sqlite_names() {
    exercise(":memory:")
}
#[cfg(feature = "postgres")]
#[test]
fn postgres_names() {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        exercise(&url)
    }
}
