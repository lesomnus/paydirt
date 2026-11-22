use super::*;
use crate::{
    driver::{Bind, Query},
    value::encoded,
};
use serde_json::json;

fn exercise(url: &str) {
    let mut db = Store::open(
        url,
        include_str!("../../generated/storage/model.json"),
        include_str!("../../generated/storage/policies.json"),
    )
    .unwrap();
    let tenant = paydirt::mint(1);
    let actor = paydirt::mint(2);
    let t = encoded(&tenant);
    let call = |name: &str, method: &str, request| Call {
        entity: format!("app.{name}"),
        method: method.into(),
        request,
    };
    db.call(
        &Scope::Internal,
        call(
            "Tenant",
            "Add",
            json!({"id":t,"alias":format!("tenant-{}",hex::encode(tenant))}),
        ),
    )
    .unwrap();
    let frame = Frame {
        address: None,
        account: None,
        topics: Default::default(),
        tenant: tenant.to_vec(),
        actor: Some(actor.to_vec()),
        caller: None,
        sets: None,
        tenants: None,
        caller_tenant: None,
        presence: None,
        trace: vec![1; 16],
        action: "/app.RobotService/Move".into(),
    };
    let begin = || Packet::Begin {
        id: "owner".into(),
        frame: frame.clone(),
    };
    let run = |call| Packet::Call {
        id: "owner".into(),
        call,
    };
    let finish = |commit| Packet::Finish {
        id: "owner".into(),
        commit,
    };
    let cell = paydirt::mint(10);
    let key = json!({"id":encoded(&cell)});
    let add = || {
        call(
            "Cell",
            "Add",
            json!({"tenant":{"id":t},"id":key["id"],"alias":"shared"}),
        )
    };
    let get = || call("Cell", "Get", json!({"ref":key}));
    fn trail(db: &mut Store, table: &str, id: &[u8]) -> Vec<String> {
        let driver = db.db.driver().unwrap();
        let mut q = Query::new("");
        let param = q.param(Bind::Bytes(id.to_vec()), driver.dialect());
        q.sql = format!("SELECT f10 AS value FROM \"pd_app__{table}\" WHERE f11={param}");
        driver.rows(q).unwrap()
    }
    db.handle(begin()).unwrap();
    db.handle(run(add())).unwrap();
    assert_eq!(
        db.handle(Packet::Call {
            id: "other".into(),
            call: get()
        })
        .unwrap_err()
        .code,
        "permission_denied"
    );
    assert_eq!(db.handle(run(add())).unwrap_err().code, "already_exists");
    assert_eq!(db.handle(finish(true)).unwrap_err().code, "already_exists");
    assert_eq!(
        db.call(&Scope::Internal, get()).unwrap_err().code,
        "not_found"
    );
    for table in ["Audit", "Outbox"] {
        assert!(trail(&mut db, table, &cell).is_empty());
    }
    db.handle(begin()).unwrap();
    db.handle(run(add())).unwrap();
    db.handle(finish(false)).unwrap();
    assert_eq!(
        db.call(&Scope::Internal, get()).unwrap_err().code,
        "not_found"
    );
    db.handle(begin()).unwrap();
    db.handle(run(add())).unwrap();
    db.handle(finish(true)).unwrap();
    assert_eq!(db.call(&Scope::Internal, get()).unwrap()["alias"], "shared");
    assert_eq!(trail(&mut db, "Audit", &cell), [frame.action]);
    assert_eq!(trail(&mut db, "Outbox", &cell), ["/app.CellService/Add"]);
}

#[cfg(feature = "sqlite")]
#[test]
fn atomic_sqlite() {
    exercise(":memory:")
}
#[cfg(feature = "postgres")]
#[test]
fn atomic_postgres() {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        exercise(&url)
    }
}

fn exercise_label_filters(url: &str) {
    let mut db = Store::open(
        url,
        include_str!("../../generated/storage/model.json"),
        include_str!("../../generated/storage/policies.json"),
    )
    .unwrap();
    let mut call = |scope: &Scope, entity: &str, method: &str, request: serde_json::Value| {
        db.call(
            scope,
            Call {
                entity: format!("app.{entity}"),
                method: method.into(),
                request,
            },
        )
    };
    let tenant = paydirt::mint(1);
    let scope = Scope::Tenant(tenant.to_vec());
    let tenant_ref = json!({"id":encoded(&tenant)});
    call(
        &Scope::Internal,
        "Tenant",
        "Add",
        json!({"id":encoded(&tenant),"alias":format!("org-{}",hex::encode(tenant))}),
    )
    .unwrap();
    for (alias, labels) in [
        ("one", json!({"team":"core", "tier":"gold"})),
        ("two", json!({"team":"core"})),
        ("other", json!({"team":"edge","tier":"gold"})),
        ("blank", json!({"team":""})),
        ("bare", json!({})),
    ] {
        call(
            &scope,
            "Holder",
            "Add",
            json!({"tenant":tenant_ref,"alias":alias,"labels":labels}),
        )
        .unwrap();
    }
    let mut list = |filter, after: &str, size| {
        call(
            &scope,
            "Holder",
            "List",
            json!({"filters":[filter],"size":size,"after":after}),
        )
    };
    let one = list(json!({"labels":{"team":"core"}}), "", 1).unwrap();
    let two = list(
        json!({"labels":{"team":"core"}}),
        one["next"].as_str().filter(|s| !s.is_empty()).unwrap(),
        1,
    )
    .unwrap();
    let mut names = [
        one["items"][0]["alias"].clone(),
        two["items"][0]["alias"].clone(),
    ];
    names.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    assert_eq!(names, [json!("one"), json!("two")]);
    for (labels, expected) in [
        (json!({"team":"core","tier":"gold"}), "one"),
        (json!({"team":""}), "blank"),
    ] {
        let rows = list(json!({"labels":labels}), "", 50).unwrap();
        assert_eq!(rows["items"].as_array().unwrap().len(), 1);
        assert_eq!(rows["items"][0]["alias"], expected);
    }
    assert_eq!(
        list(json!({"labels":{}}), "", 50).unwrap_err().code,
        "invalid_argument"
    );
    assert_eq!(
        list(json!({"tenant":tenant_ref,"labels":{}}), "", 50).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}

#[cfg(feature = "sqlite")]
#[test]
fn sqlite_label_filters() {
    exercise_label_filters(":memory:")
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_label_filters() {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        exercise_label_filters(&url);
    }
}
