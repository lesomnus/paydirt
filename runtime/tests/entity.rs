#![cfg(any(feature = "sqlite", feature = "postgres"))]
use base64::{Engine, engine::general_purpose::STANDARD};
use paydirt_store::{Call, Scope, Store};
use serde_json::{Value, json};

fn op(entity: &str, method: &str, request: Value) -> Call {
    Call {
        entity: format!("app.{entity}"),
        method: method.into(),
        request,
    }
}
fn exercise(url: &str) {
    let mut db = Store::open(
        url,
        include_str!("../../generated/storage/model.json"),
        include_str!("../../generated/storage/policies.json"),
    )
    .unwrap();
    let tenant = paydirt::mint(1);
    let other = paydirt::mint(1);
    for id in [&tenant, &other] {
        db.call(
            &Scope::Internal,
            op(
                "Tenant",
                "Add",
                json!({"id":STANDARD.encode(id),"alias":format!("tenant-{}",hex::encode(id))}),
            ),
        )
        .unwrap();
    }
    let this = Scope::Tenant(tenant.to_vec());
    let foreign = Scope::Tenant(other.to_vec());
    let t = json!({"id":STANDARD.encode(tenant)});
    let cell = db
        .call(&this, op("Cell", "Add", json!({"tenant":t,"alias":"dock"})))
        .unwrap();
    let key = json!({"id":cell["id"]});
    assert!(paydirt::has_domain(
        &STANDARD.decode(key["id"].as_str().unwrap()).unwrap(),
        10
    ));
    let get = || op("Cell", "Get", json!({"ref":key}));
    assert_eq!(db.call(&this, get()).unwrap()["alias"], "dock");
    assert_eq!(db.call(&foreign, get()).unwrap_err().code, "not_found");
    assert_eq!(
        db.call(
            &foreign,
            op("Cell", "Patch", json!({"ref":key,"alias":"foreign"}))
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    assert_eq!(
        db.call(&foreign, op("Cell", "Erase", key.clone())).unwrap()["erased"],
        false
    );
    assert_eq!(
        db.call(
            &foreign,
            op("Cell", "Add", json!({"tenant":t,"alias":"foreign"}))
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    assert_eq!(
        db.call(&this, op("Cell", "Patch", json!({"ref":key})))
            .unwrap()["alias"],
        "dock"
    );
    assert_eq!(
        db.call(&this, op("Cell", "Patch", json!({"ref":key,"alias":""})))
            .unwrap_err()
            .field
            .as_deref(),
        Some("alias")
    );
    let add = |alias: &str| json!({"tenant":t,"alias":alias});
    let r = db.call(&this, op("Robot", "Add", add("r1"))).unwrap();
    let rkey = json!({"id":r["id"]});
    assert_eq!(
        db.call(
            &this,
            op(
                "Robot",
                "Patch",
                json!({"ref":rkey,"dateUpdated":r["dateUpdated"],"cell":key})
            )
        )
        .unwrap()["cell"]["id"],
        key["id"]
    );
    assert_eq!(
        db.call(
            &this,
            op(
                "Robot",
                "Patch",
                json!({"ref":rkey,"dateUpdated":r["dateUpdated"],"alias":"race"})
            )
        )
        .unwrap_err()
        .code,
        "aborted"
    );
    let p = db
        .call(
            &this,
            op(
                "Robot",
                "Patch",
                json!({"ref":rkey,"dateUpdatedForce":true,"cellNull":true}),
            ),
        )
        .unwrap();
    assert_eq!(p["alias"], "r1");
    assert!(p.get("cell").is_none());
    let batch = json!({"items":[add("rolled-back"),add("r1")]});
    assert_eq!(
        db.call(&this, op("Robot", "AddMany", batch))
            .unwrap_err()
            .code,
        "already_exists"
    );
    assert_eq!(
        db.call(
            &this,
            op(
                "Robot",
                "Get",
                json!({"ref":{"slug":{"tenant":t,"alias":"rolled-back"}}})
            )
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    assert_eq!(
        db.call(&this, op("Robot", "Erase", rkey)).unwrap()["erased"],
        true
    );
    db.call(&this, op("Robot", "Add", add("r1"))).unwrap();
    assert_eq!(
        db.call(&this, op("Cell", "Erase", key.clone())).unwrap()["erased"],
        true
    );
    assert_eq!(
        db.call(&this, op("Cell", "Erase", key)).unwrap()["erased"],
        false
    );
    assert_eq!(
        db.call(
            &Scope::Internal,
            op("Tenant", "Erase", json!({"id":STANDARD.encode(other)}))
        )
        .unwrap()["erased"],
        true
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
