#![cfg(all(not(target_arch = "wasm32"), feature = "sqlite"))]
use base64::{Engine, engine::general_purpose::STANDARD};
use paydirt_store::{Call, Frame, Packet, Scope, Store, archive_files::Archive, retention::Report};
use serde_json::{Value, json};
use std::{fs, path::Path};
const MODEL: &str = include_str!("../../generated/storage/model.json");
const POLICIES: &str = include_str!("../../generated/storage/policies.json");
fn open(url: &str) -> Store {
    Store::open(url, MODEL, POLICIES)
        .unwrap()
        .with_trail(
            Default::default(),
            include_bytes!("../../generated/storage/descriptors.binpb"),
            include_str!("../../generated/storage/admission.json"),
        )
        .unwrap()
}
fn sample(store: &mut Store) -> (String, String) {
    let tenant = paydirt::mint(1);
    let t = STANDARD.encode(tenant);
    store
        .call(
            &Scope::Internal,
            Call {
                entity: "app.Tenant".into(),
                method: "Add".into(),
                request: json!({"id":t,"alias":format!("t{}",hex::encode(tenant))}),
            },
        )
        .unwrap();
    let frame: Frame = serde_json::from_value(
        json!({"tenant":tenant,"actor":paydirt::mint(2),"action":"/app.CellService/Add"}),
    )
    .unwrap();
    store
        .handle(Packet::Begin {
            id: "r".into(),
            frame,
        })
        .unwrap();
    let row = store
        .handle(Packet::Call {
            id: "r".into(),
            call: Call {
                entity: "app.Cell".into(),
                method: "Add".into(),
                request: json!({"tenant":{"id":t},"alias":"original-vault-content"}),
            },
        })
        .unwrap();
    store
        .handle(Packet::Finish {
            id: "r".into(),
            commit: true,
        })
        .unwrap();
    (
        paydirt::format_id(&tenant).unwrap(),
        paydirt::format_id(&STANDARD.decode(row["id"].as_str().unwrap()).unwrap()).unwrap(),
    )
}
fn pass(
    store: &mut Store,
    vault: &Archive,
    to: &Archive,
    tenant: &str,
    mut req: Value,
) -> paydirt_store::Result<Report> {
    req["tenant"] = json!(tenant);
    store.manage_trail(vault, Some(to), serde_json::from_value(req).unwrap())
}
fn exercise(url: &str, dir: &Path) {
    let mut store = open(url);
    let (tenant, object) = sample(&mut store);
    let (other, _) = sample(&mut store);
    let folder = dir.join("vault");
    let vault = Archive::open(&folder, &[17; 32]).unwrap();
    let destination = dir.join("export");
    let recipient = Archive::open(&destination, &[44; 32]).unwrap();
    let vault = vault.with_notary(paydirt_store::attestation::Notary::new(
        Box::new(paydirt_store::attestation::Ed25519::from_seed(&[72; 32]).unwrap()),
        Default::default(),
        Box::new(paydirt_store::attestation::DirectoryWitness(
            dir.join("witness"),
        )),
    ));
    let run = |store: &mut Store, t: &str, req| pass(store, &vault, &recipient, t, req).unwrap();
    run(
        &mut store,
        &tenant,
        json!({"action":"policy","policy":{"by":{"app.Cell":{"retain_ms":0,"discard":true}},"held":["app.Cell"]}}),
    );
    let dry = run(
        &mut store,
        &tenant,
        json!({"action":"collect","dry_run":true}),
    );
    assert_eq!(dry.archived.len(), 1);
    assert_eq!(dry.archived, dry.held);
    assert!(!folder.exists());
    fs::write(&folder, "simulated storage outage").unwrap();
    assert!(
        pass(
            &mut store,
            &vault,
            &recipient,
            &tenant,
            json!({"action":"collect"})
        )
        .is_err()
    );
    fs::remove_file(&folder).unwrap();
    assert_eq!(
        run(&mut store, &tenant, json!({"action":"inspect"}))
            .inventory
            .values()
            .next()
            .unwrap(),
        "online"
    );
    assert_eq!(
        dry.archived,
        run(&mut store, &tenant, json!({"action":"collect"})).archived
    );
    assert_eq!(store.attest_trail(&vault, true).unwrap().unwrap().k, 1);
    assert!(store.attest_trail(&vault, false).is_ok());
    let inspect = run(&mut store, &other, json!({"action":"inspect"}));
    assert_eq!(inspect.inventory.values().next().unwrap(), "online");
    assert!(
        run(&mut store, &tenant, json!({"action":"purge"}))
            .removed
            .is_empty()
    );
    let held = run(
        &mut store,
        &tenant,
        json!({"action":"forget","subject":object}),
    );
    assert_eq!(held.owed, [object.as_str()]);
    assert_eq!(held.held, dry.held);
    let retired = run(&mut store, &tenant, json!({"action":"retire"}));
    assert!(retired.retired);
    assert_eq!(retired.held, dry.held);
    drop(store);
    let mut store = open(url);
    assert_eq!(
        run(&mut store, &tenant, json!({"action":"inspect"})).owed,
        [object.as_str()]
    );
    let export =
        json!({"action":"export","recipient":"reviewer","operator":"manager","reason":"case 71"});
    let bundle = run(&mut store, &tenant, export.clone()).bundle.unwrap();
    let doc: Value = serde_json::from_slice(&recipient.read(&bundle).unwrap()).unwrap();
    assert_eq!(doc["rows"].as_array().unwrap().len(), 1);
    assert_eq!(doc["tenant"], tenant);
    assert!(
        Archive::open(&destination, &[17; 32])
            .unwrap()
            .read(&bundle)
            .is_err()
    );
    assert!(
        fs::read(destination.join(bundle))
            .unwrap()
            .windows(8)
            .all(|s| s != b"reviewer")
    );
    run(
        &mut store,
        &tenant,
        json!({"action":"policy","policy":{"default":{"retain_ms":100000}}}),
    );
    assert_eq!(
        run(&mut store, &tenant, json!({"action":"retry"})).blanked,
        dry.held
    );
    assert!(
        run(&mut store, &tenant, json!({"action":"retry"}))
            .blanked
            .is_empty()
    );
    let bundle = run(&mut store, &tenant, export).bundle.unwrap();
    let doc: Value = serde_json::from_slice(&recipient.read(&bundle).unwrap()).unwrap();
    assert_eq!(doc["rows"][0]["patch"], "");
    let preview = run(
        &mut store,
        &tenant,
        json!({"action":"purge","dry_run":true}),
    );
    assert_eq!(
        preview.removed,
        run(&mut store, &tenant, json!({"action":"purge"})).removed
    );
    assert_eq!(
        fs::read_dir(folder)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().unwrap().is_file())
            .count(),
        0
    );
    store.attest_trail(&vault, true).unwrap();
    store.attest_trail(&vault, false).unwrap();
    assert_eq!(
        run(&mut store, &other, json!({"action":"inspect"}))
            .inventory
            .len(),
        1
    );
}
#[test]
fn dry_run_recovery_holds_and_encrypted_export() {
    backends(exercise);
}
#[test]
fn shared_owners_and_transactional_obligations() {
    backends(shared_window);
}
fn backends(exercise: fn(&str, &Path)) {
    let temp = tempfile::tempdir().unwrap();
    exercise(temp.path().join("db").to_str().unwrap(), temp.path());
    #[cfg(feature = "postgres")]
    if let Ok(url) = std::env::var("DATABASE_URL") {
        use diesel::{Connection, connection::SimpleConnection};
        let mut admin = diesel::PgConnection::establish(&url).unwrap();
        let schema = format!("retention_{}", hex::encode(paydirt::mint(0)));
        admin
            .batch_execute(&format!("CREATE SCHEMA {schema}"))
            .unwrap();
        let scoped = format!(
            "{url}{}options=-csearch_path%3D{schema}",
            if url.contains('?') { "&" } else { "?" }
        );
        let tmp = tempfile::tempdir().unwrap();
        let result = std::panic::catch_unwind(|| exercise(&scoped, tmp.path()));
        admin
            .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
            .unwrap();
        if let Err(err) = result {
            std::panic::resume_unwind(err)
        }
    }
}

fn shared_window(url: &str, dir: &Path) {
    let mut policies: Value = serde_json::from_str(POLICIES).unwrap();
    policies["app.Cell"]["entity"]["formerly"] = json!([{"message":"Box","name":"box"}]);
    let mut db = Store::open(url, MODEL, &policies.to_string()).unwrap();
    let vault = Archive::open(dir.join("shared-vault"), &[88; 32]).unwrap();
    let recipient = Archive::open(dir.join("recipient"), &[99; 32]).unwrap();
    let left = paydirt::mint(1);
    let right = paydirt::mint(1);
    let outsider = paydirt::mint(1);
    let uuid = |id: &[u8]| paydirt::format_id(id).unwrap();
    let id = paydirt::mint(
        policies["app.Audit"]["entity"]["domain"]
            .as_u64()
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let subject = paydirt::mint(10);
    db.seed(vec![("app.Audit".into(), json!({
        "id":STANDARD.encode(id),"tenantId":STANDARD.encode(left),"actorTenantId":STANDARD.encode(right),
        "actorId":STANDARD.encode(paydirt::mint(2)),"objectId":STANDARD.encode(subject),
        "action":"/app.CellService/Add","domain":10,"dateCreated":"2020-01-01T00:00:00Z","value":STANDARD.encode(b"private")
    }))]).unwrap();
    let pass = |db: &mut Store, owner: &[u8], arg| pass(db, &vault, &recipient, &uuid(owner), arg);
    let set = |by: Value| json!({"action":"policy","policy":{"by":by}});
    for kind in ["not-a-resource", "cells"] {
        assert!(pass(&mut db, &left, set(json!({kind:{}}))).is_err());
    }
    assert!(pass(&mut db, &left, set(json!({"box":{},"cell":{}}))).is_err());
    let policy = pass(
        &mut db,
        &left,
        set(json!({"box":{"retain_ms":0,"discard":true}})),
    )
    .unwrap()
    .policy;
    assert!(policy.by.contains_key("app.Cell"));
    let inspect = json!({"action":"inspect"});
    let collect = json!({"action":"collect"});
    assert!(
        pass(&mut db, &left, collect.clone())
            .unwrap()
            .archived
            .is_empty()
    );
    assert_eq!(
        pass(&mut db, &right, inspect.clone()).unwrap().inventory[&uuid(&id)],
        "online"
    );
    assert!(
        pass(&mut db, &outsider, inspect.clone())
            .unwrap()
            .inventory
            .is_empty()
    );
    pass(&mut db, &right, set(json!({"app.Cell":{"retain_ms":0}}))).unwrap();
    let collected = pass(&mut db, &left, collect.clone()).unwrap();
    assert_eq!(collected.archived, [uuid(&id)]);
    assert!(collected.removed.is_empty());
    for owner in [left, right] {
        assert_eq!(
            pass(&mut db, &owner, inspect.clone()).unwrap().inventory[&uuid(&id)],
            "archived"
        );
        let export = json!({"action":"export","operator":"test","recipient":"owner","reason":"shared record"});
        let bundle = pass(&mut db, &owner, export).unwrap().bundle.unwrap();
        let rows: Value = serde_json::from_slice(&recipient.read(&bundle).unwrap()).unwrap();
        assert_eq!(rows["rows"][0]["id"], STANDARD.encode(id));
    }
    pass(
        &mut db,
        &right,
        json!({"action":"policy","policy":{"held":["cell"]}}),
    )
    .unwrap();
    let blocked = pass(&mut db, &left, json!({"action":"purge"})).unwrap();
    assert_eq!(blocked.held, [uuid(&id)]);
    assert!(blocked.blanked.is_empty());
    pass(&mut db, &right, set(json!({}))).unwrap();
    let departure = pass(&mut db, &left, json!({"action":"purge"})).unwrap();
    assert_eq!(departure.blanked, [uuid(&id)]);
    assert!(departure.removed.is_empty());
    let bundle = pass(
        &mut db,
        &right,
        json!({"action":"export","operator":"test","recipient":"owner","reason":"after departure"}),
    )
    .unwrap()
    .bundle
    .unwrap();
    let shared: Value = serde_json::from_slice(&recipient.read(&bundle).unwrap()).unwrap();
    assert_eq!(shared["rows"][0]["id"], STANDARD.encode(id));
    assert_eq!(shared["rows"][0]["value"], "");
    let refused: paydirt_store::Result<()> = db.transaction(&Scope::Internal, |tx| {
        tx.owe_trail_erasure(left, subject)?;
        Err(paydirt_store::Fault::invalid("undo accompanying write"))
    });
    assert!(refused.is_err());
    assert!(
        pass(&mut db, &left, inspect.clone())
            .unwrap()
            .owed
            .is_empty()
    );
    let second = paydirt::mint(10);
    db.transaction(&Scope::Internal, |tx| {
        tx.owe_trail_erasure(left, subject)?;
        tx.owe_trail_erasure(left, second)?;
        tx.owe_trail_erasure(left, subject)
    })
    .unwrap();
    assert_eq!(pass(&mut db, &left, inspect).unwrap().owed.len(), 2);
    let retry = pass(&mut db, &left, json!({"action":"retry"})).unwrap();
    assert!(retry.owed.is_empty());
}
