#![cfg(any(feature = "postgres", feature = "sqlite"))]
use diesel::{Connection, connection::SimpleConnection};
use paydirt_store::{Call, Scope, Store};
use serde_json::{Value, json};

fn call(method: &str, request: Value) -> Call {
    Call {
        entity: "catalog.Part".into(),
        method: method.into(),
        request,
    }
}
fn exercise(url: &str, v1: &str, v2: &str, alter: impl FnOnce()) {
    let open = |schema: &str| Store::open(url, schema, "{}");
    let mut db = open(v1).unwrap();
    for id in [1, 2] {
        db.call(
            &Scope::Internal,
            call("Add", json!({"id":id,"label":"same"})),
        )
        .unwrap();
    }
    drop(db);
    // Adding the unique index fails on old data. The new column and the
    // migration record must roll back along with it, so the old binary works.
    assert!(open(v2).is_err());
    let mut db = open(v1).unwrap();
    db.call(
        &Scope::Internal,
        call("Patch", json!({"ref":{"id":2},"label":"different"})),
    )
    .unwrap();
    drop(db);
    let mut db = open(v2).unwrap();
    let key = json!({"ref":{"id":1}});
    let row = db.call(&Scope::Internal, call("Get", key.clone())).unwrap();
    assert_eq!(row["name"], "same");
    assert!(row.get("label").is_none() && row.get("note").is_none());
    db.call(
        &Scope::Internal,
        call("Patch", json!({"ref":{"id":1},"note":"kept"})),
    )
    .unwrap();
    drop(db);
    assert!(
        open(v1).is_err(),
        "old generator must refuse to remove the new field"
    );
    assert_eq!(
        open(v2)
            .unwrap()
            .call(&Scope::Internal, call("Get", key))
            .unwrap()["note"],
        "kept"
    );
    let mut newer: Value = serde_json::from_str(v2).unwrap();
    newer["version"] = json!(99);
    assert!(open(&newer.to_string()).is_err());
    alter();
    assert!(
        matches!(open(v2),Err(e) if e.code=="failed_precondition" && e.message.contains("drifted"))
    );
}
fn inputs() -> Option<(String, String)> {
    let read = |k| {
        std::env::var(k)
            .ok()
            .map(|p| std::fs::read_to_string(p).unwrap())
    };
    Some((read("PAYDIRT_V1")?, read("PAYDIRT_V2")?))
}
#[cfg(feature = "sqlite")]
#[test]
fn sqlite() {
    let Some((v1, v2)) = inputs() else { return };
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("upgrade.db");
    let url = file.to_str().unwrap();
    exercise(url, &v1, &v2, || {
        diesel::SqliteConnection::establish(url)
            .unwrap()
            .batch_execute("ALTER TABLE \"pd_catalog__Part\" ADD COLUMN unexpected TEXT")
            .unwrap();
    });
}
#[cfg(feature = "postgres")]
#[test]
fn postgres() {
    let Some((v1, v2)) = inputs() else { return };
    let url = std::env::var("DATABASE_URL").expect("disposable PostgreSQL URL");
    let schema = format!("upgrade_{}", hex::encode(paydirt::mint(1)));
    let mut admin = diesel::PgConnection::establish(&url).unwrap();
    admin
        .batch_execute(&format!("CREATE SCHEMA {schema}"))
        .unwrap();
    let scoped = format!(
        "{url}{}options=-csearch_path%3D{schema}",
        if url.contains('?') { "&" } else { "?" }
    );
    let test = std::panic::catch_unwind(|| {
        exercise(&scoped, &v1, &v2, || {
            diesel::PgConnection::establish(&scoped)
                .unwrap()
                .batch_execute("ALTER TABLE \"pd_catalog__Part\" ADD COLUMN unexpected TEXT")
                .unwrap();
        })
    });
    admin
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .unwrap();
    if let Err(panic) = test {
        std::panic::resume_unwind(panic)
    }
}

fn former_layout(url: &str) {
    let earlier = json!({"version":1,"entities":[
        {"name":"catalog.Part","key":"id","fields":[{"name":"id","json":"id","number":1,"kind":"INT32"},{"name":"label","json":"label","number":2,"kind":"STRING","unique":true}],"indexes":[]},
        {"name":"catalog.Line","key":"id","fields":[{"name":"id","json":"id","number":1,"kind":"INT32"},{"name":"part","json":"part","number":2,"kind":"MESSAGE","target":"catalog.Part","nullable":true}],"indexes":[]}
    ]}).to_string();
    let new = earlier.replace("catalog.Part", "catalog.Component");
    let policies = r#"{"catalog.Component":{"entity":{"formerly":[{"message":"Part"}]}}}"#;
    let op = |db: &mut Store, name: &str, method: &str, request| {
        db.call(
            &Scope::Internal,
            Call {
                entity: name.into(),
                method: method.into(),
                request,
            },
        )
    };
    let mut db = Store::open(url, &earlier, "{}").unwrap();
    op(
        &mut db,
        "catalog.Part",
        "Add",
        json!({"id":1,"label":"bolt"}),
    )
    .unwrap();
    op(
        &mut db,
        "catalog.Part",
        "Add",
        json!({"id":2,"label":"nut"}),
    )
    .unwrap();
    op(
        &mut db,
        "catalog.Line",
        "Add",
        json!({"id":3,"part":{"id":1}}),
    )
    .unwrap();
    drop(db);
    assert!(Store::open(url, &new, "{}").is_err());
    let mut db = Store::open(url, &new, policies).unwrap();
    assert_eq!(
        op(&mut db, "catalog.Component", "Get", json!({"ref":{"id":1}})).unwrap()["label"],
        "bolt"
    );
    assert_eq!(
        op(&mut db, "catalog.Line", "Get", json!({"ref":{"id":3}})).unwrap()["part"]["id"],
        1
    );
    assert_eq!(
        op(
            &mut db,
            "catalog.Component",
            "Add",
            json!({"id":4,"label":"bolt"})
        )
        .unwrap_err()
        .code,
        "already_exists"
    );
    assert!(op(&mut db, "catalog.Component", "Erase", json!({"id":1})).is_err());
    drop(db);
    assert!(Store::open(url, &earlier, "{}").is_err());
    Store::open(url, &new, policies).unwrap();
    if url.starts_with("postgres") {
        #[cfg(feature = "postgres")]
        {
            let mut admin = diesel::PgConnection::establish(url).unwrap();
            admin.batch_execute("ALTER TABLE \"pd_catalog__Component\" ADD COLUMN legacy_label TEXT NOT NULL DEFAULT ''").unwrap();
            admin
                .batch_execute(
                    "ALTER TABLE \"pd_catalog__Component\" ALTER COLUMN legacy_label DROP DEFAULT",
                )
                .unwrap();
            assert!(
                matches!(Store::open(url,&new,policies),Err(e) if e.message.contains("legacy_label")&&e.message.contains("no default"))
            );
            admin
                .batch_execute("ALTER TABLE \"pd_catalog__Component\" DROP COLUMN legacy_label")
                .unwrap();
            assert!(admin.batch_execute("CREATE UNIQUE INDEX CONCURRENTLY broken_manual ON \"pd_catalog__Component\" ((1))").is_err());
            Store::open(url, &new, policies).unwrap();
            admin
                .batch_execute("DROP INDEX \"pd_catalog__Component__field_2\"")
                .unwrap();
            assert!(admin.batch_execute("CREATE UNIQUE INDEX CONCURRENTLY \"pd_catalog__Component__field_2\" ON \"pd_catalog__Component\" ((1))").is_err());
            assert!(
                matches!(Store::open(url,&new,policies),Err(e) if e.message.contains("pd_catalog__Component__field_2")&&e.message.contains("invalid or unready"))
            );
        }
    }
}
#[cfg(feature = "sqlite")]
#[test]
fn formerly_sqlite() {
    let tmp = tempfile::tempdir().unwrap();
    former_layout(tmp.path().join("old.db").to_str().unwrap());
}
#[cfg(feature = "postgres")]
#[test]
fn formerly_postgres() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let schema = format!("former_{}", hex::encode(paydirt::mint(1)));
    let mut conn = diesel::PgConnection::establish(&url).unwrap();
    conn.batch_execute(&format!("CREATE SCHEMA {schema}"))
        .unwrap();
    let scoped = format!(
        "{url}{}options=-csearch_path%3D{schema}",
        if url.contains('?') { "&" } else { "?" }
    );
    let run = std::panic::catch_unwind(|| former_layout(&scoped));
    conn.batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .unwrap();
    if let Err(e) = run {
        std::panic::resume_unwind(e);
    }
}
