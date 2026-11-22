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
        call("Tenant", "Add", json!({"id":t,"alias":hex::encode(tenant)})),
    )
    .unwrap();
    let frame = Frame {
        account: None,
        topics: Default::default(),
        tenant: tenant.to_vec(),
        actor: Some(actor.to_vec()),
        caller: None,
        sets: None,
        tenants: None,
        caller_tenant: None,
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
