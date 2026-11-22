#![cfg(not(target_arch = "wasm32"))]
use base64::{Engine, engine::general_purpose::STANDARD};
use paydirt_store::{
    attestation::{Blob, DirectoryWitness, Ed25519, Head, Notary, Signature, manifest_statement},
    integrity::{Keys, Line, Verdict},
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
fn notary(witness: std::path::PathBuf) -> Notary {
    Notary::new(
        Box::new(Ed25519::from_seed(&[23; 32]).unwrap()),
        BTreeSet::new(),
        Box::new(DirectoryWitness(witness)),
    )
}
#[test]
fn accepts_go_digests_seals_manifest_and_gzip_chain() {
    let raw: Value =
        serde_json::from_str(include_str!("../../tests/trail/go-vectors.json")).unwrap();
    let bytes = |n: &str| STANDARD.decode(raw[n].as_str().unwrap()).unwrap();
    let keys = Keys::new(bytes("key"), vec![], Default::default()).unwrap();
    let obj = bytes("object").try_into().unwrap();
    let line = Line {
        id: bytes("line").try_into().unwrap(),
        micros: raw["micros"].as_i64().unwrap(),
        action: raw["action"].as_str().unwrap(),
    };
    let wire = bytes("wire");
    let digest = keys.sign(&obj, &line, Some((&[1], &wire)), true);
    assert_eq!(digest, bytes("digest"));
    assert_eq!(keys.sign(&obj, &line, None, false), bytes("gone"));
    assert_eq!(keys.seal(&obj, &line, &[1], &wire), bytes("seal"));
    assert_eq!(keys.forgotten(&obj), bytes("forgotten"));
    let keys = Keys::new(vec![9; 32], vec![bytes("key")], Default::default()).unwrap();
    assert_eq!(
        keys.check(&digest, &obj, &line, Some((&[1], &wire)), false),
        Verdict::Matches
    );
    assert_eq!(
        keys.check(&digest, &obj, &line, Some((&[1], b"altered")), false),
        Verdict::Differs
    );
    let mut damaged = digest;
    damaged[20] ^= 1;
    assert_eq!(
        keys.check(&damaged, &obj, &line, Some((&[1], &wire)), false),
        Verdict::Forged
    );
    let tmp = tempfile::tempdir().unwrap();
    let n = notary(tmp.path().into());
    let statement = manifest_statement(
        "row-1",
        "tenant",
        &format!(
            "sha256:{}",
            paydirt_store::archive_files::Archive::digest(&wire)
        ),
        "archive",
        "present",
        &BTreeMap::from([
            ("owner".into(), "tenant".into()),
            ("z".into(), "한글".into()),
        ]),
    );
    assert_eq!(statement, bytes("statement"));
    let sig = Signature {
        key: raw["pk"].as_str().unwrap().into(),
        sig: raw["sig"].as_str().unwrap().into(),
    };
    n.verify(&statement, &sig).unwrap();
    assert!(n.verify(b"altered", &sig).is_err());
    let points: Vec<_> = raw["checkpoints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| STANDARD.decode(s.as_str().unwrap()).unwrap())
        .collect();
    n.verify_point(&points[0], None).unwrap();
    let head = Head {
        k: 1,
        digest: format!(
            "sha256:{}",
            paydirt_store::archive_files::Archive::digest(&points[0])
        ),
    };
    n.verify_point(&points[1], Some(&head)).unwrap();
    assert!(n.verify_point(&points[1], None).is_err());
}
#[test]
fn local_checkpoint_is_witnessed_and_removal_needs_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("store");
    let wit = tmp.path().join("witness");
    let n = notary(wit.clone());
    let blob = Blob::of("demo", &"ab".repeat(32));
    let one = BTreeSet::from([blob.clone()]);
    let first = n
        .check_chain(&root, one.clone(), Default::default(), true)
        .unwrap()
        .unwrap();
    assert_eq!(
        n.check_chain(&root, one.clone(), Default::default(), false)
            .unwrap(),
        Some(first)
    );
    assert!(
        n.check_chain(&root, Default::default(), Default::default(), true)
            .is_err()
    );
    let second = n
        .check_chain(&root, Default::default(), one, true)
        .unwrap()
        .unwrap();
    assert_eq!(second.k, 2);
    std::fs::remove_file(root.join(".checkpoints/00000000000000000002.gz")).unwrap();
    assert!(
        n.check_chain(&root, Default::default(), Default::default(), false)
            .is_err()
    );
}
#[cfg(feature = "sqlite")]
#[test]
fn transaction_seals_current_personal_row_and_detects_a_changed_value() {
    use diesel::{Connection, connection::SimpleConnection};
    use paydirt_store::{Call, Frame, Packet, Scope, Store};
    use serde_json::json;
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("app.db");
    let path = file.to_str().unwrap();
    let mut db = Store::open(
        path,
        include_str!("../../generated/storage/model.json"),
        include_str!("../../generated/storage/policies.json"),
    )
    .unwrap()
    .with_trail(
        Default::default(),
        include_bytes!("../../generated/storage/descriptors.binpb"),
        include_str!("../../generated/storage/admission.json"),
    )
    .unwrap()
    .with_audit_keys(Keys::new(vec![7; 32], vec![], Default::default()).unwrap())
    .unwrap();
    let t = paydirt::mint(1);
    let tenant = STANDARD.encode(t);
    db.call(
        &Scope::Internal,
        Call {
            entity: "app.Tenant".into(),
            method: "Add".into(),
            request: json!({"alias":"work","id":tenant}),
        },
    )
    .unwrap();
    let f: Frame = serde_json::from_value(
        json!({"actor":paydirt::mint(2),"tenant":t,"action":"/app.HolderService/Add"}),
    )
    .unwrap();
    let mut insert = |alias: &str, commit| {
        db.handle(Packet::Begin {
            id: "r".into(),
            frame: f.clone(),
        })
        .unwrap();
        let r = db
            .handle(Packet::Call {
                id: "r".into(),
                call: Call {
                    entity: "app.Holder".into(),
                    method: "Add".into(),
                    request: json!({"tenant":{"id":tenant},"alias":alias,"name":"Original"}),
                },
            })
            .unwrap();
        db.handle(Packet::Finish {
            id: "r".into(),
            commit,
        })
        .unwrap();
        STANDARD
            .decode(r["id"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap()
    };
    let gone = insert("discarded", false);
    let id = insert("writer", true);
    assert!(db.verify_object("app.Holder", gone).is_err());
    let check = db.verify_object("app.Holder", id).unwrap();
    assert_eq!(check.digest, Verdict::Matches);
    assert_eq!(check.seal, Some(Verdict::Matches));
    let mut sql = diesel::SqliteConnection::establish(path).unwrap();
    sql.batch_execute("UPDATE pd_app__Holder SET f5='Changed outside the recorder'")
        .unwrap();
    let check = db.verify_object("app.Holder", id).unwrap();
    assert_eq!(check.digest, Verdict::Matches);
    assert_eq!(check.seal, Some(Verdict::Differs));
    sql.batch_execute("UPDATE pd_app__Audit SET f10='another call'")
        .unwrap();
    assert_eq!(
        db.verify_object("app.Holder", id).unwrap().digest,
        Verdict::Forged
    );
}

#[cfg(feature = "sqlite")]
fn check_database(url: &str, directory: &std::path::Path, raw: impl Fn(&str)) {
    use paydirt_store::{Call, Fault, Scope, Store, archive_files::Archive};
    use serde_json::json;
    let open = |key: u8| {
        Store::open(
            url,
            include_str!("../../generated/storage/model.json"),
            include_str!("../../generated/storage/policies.json"),
        )
        .unwrap()
        .with_trail(
            Default::default(),
            include_bytes!("../../generated/storage/descriptors.binpb"),
            include_str!("../../generated/storage/admission.json"),
        )
        .unwrap()
        .with_audit_keys(Keys::new(vec![key; 32], vec![], Default::default()).unwrap())
        .unwrap()
    };
    let mut db = open(6);
    let t = paydirt::mint(1);
    let object = paydirt::mint(10);
    let reference = paydirt::format_id(&object).unwrap();
    db.call(
        &Scope::Internal,
        Call {
            entity: "app.Tenant".into(),
            method: "Add".into(),
            request: json!({"id":STANDARD.encode(t),"alias":"own"}),
        },
    )
    .unwrap();
    db.seed(vec![(
        "app.Cell".into(),
        json!({"id":STANDARD.encode(object),"tenant":{"id":STANDARD.encode(t)},"alias":"external"}),
    )])
    .unwrap();
    let scan =
        |db: &mut Store, archive: Option<&Archive>| db.check_kind("app.Cell", archive).unwrap();
    assert_eq!(
        scan(&mut db, None).findings[&reference].digest,
        Verdict::NotOnTrail
    );
    let rollback: paydirt_store::Result<()> = db.transaction(&Scope::Internal, |tx| {
        tx.record_objects("app.Cell", &[object], "/ops/first", false)?;
        Err(Fault::invalid("rollback"))
    });
    assert!(rollback.is_err());
    assert_eq!(
        scan(&mut db, None).findings[&reference].digest,
        Verdict::NotOnTrail
    );
    assert_eq!(
        db.transaction(&Scope::Internal, |tx| tx.record_objects(
            "app.Cell",
            &[object, object],
            "/ops/admit",
            false
        ))
        .unwrap(),
        1
    );
    assert!(scan(&mut db, None).findings.is_empty());
    raw("UPDATE \"pd_app__Cell\" SET f4='external-edit' WHERE f4='external'");
    assert_eq!(
        scan(&mut db, None).findings[&reference].digest,
        Verdict::Differs
    );
    assert!(
        db.transaction(&Scope::Internal, |tx| tx.record_objects(
            "app.Cell",
            &[object],
            "/ops/erase",
            true
        ))
        .is_err()
    );
    db.transaction(&Scope::Internal, |tx| {
        tx.record_objects("app.Cell", &[object], "/ops/accept", false)
    })
    .unwrap();
    assert!(scan(&mut db, None).findings.is_empty());
    let bad = db.transaction(&Scope::Internal, |tx| {
        tx.record_objects("app.Cell", &[object], "", false)
    });
    assert!(bad.is_err());
    let missing = db.transaction(&Scope::Internal, |tx| {
        tx.record_objects("app.Cell", &[paydirt::mint(10)], "/ops/record", false)
    });
    assert!(missing.is_err());
    let mut future = open(6)
        .with_audit_keys(
            Keys::new(vec![6; 32], vec![], Default::default())
                .unwrap()
                .since(chrono::Utc::now() + chrono::TimeDelta::days(1)),
        )
        .unwrap();
    assert!(
        future
            .check_kind("app.Cell", None)
            .unwrap_err()
            .message
            .contains("audit.digest.since")
    );
    let mut wrong = open(121);
    let original = paydirt_store::integrity::fingerprint(&[6; 32]);
    assert_eq!(
        scan(&mut wrong, None).unheld,
        BTreeSet::from([hex::encode(original)])
    );
    let mut retired = open(121)
        .with_audit_keys(Keys::new(vec![121; 32], vec![], BTreeSet::from([original])).unwrap())
        .unwrap();
    let report = scan(&mut retired, None);
    assert_eq!(report.retired, 1);
    assert!(report.findings.is_empty());
    assert_eq!(
        scan(&mut wrong, None).findings[&reference].digest,
        Verdict::UnknownKey
    );
    raw(
        "UPDATE \"pd_app__Audit\" SET f21=(SELECT f21 FROM \"pd_app__Audit\" WHERE f10='/ops/admit') WHERE f10='/ops/accept'",
    );
    assert_eq!(
        scan(&mut db, None).findings[&reference].digest,
        Verdict::Forged
    );
    db.transaction(&Scope::Internal, |tx| {
        tx.record_objects("app.Cell", &[object], "/ops/review", false)
    })
    .unwrap();
    let archive_dir = directory.join("archive");
    let witness = directory.join("witness");
    let vault = Archive::open(&archive_dir, &[54; 32])
        .unwrap()
        .with_notary(notary(witness.clone()));
    let verify = Archive::without_decryption_key(&archive_dir).with_notary(notary(witness));
    let tenant = paydirt::format_id(&t).unwrap();
    for req in [
        json!({"tenant":tenant,"action":"policy","policy":{"by":{"app.Cell":{"retain_ms":0}}}}),
        json!({"tenant":tenant,"action":"collect"}),
    ] {
        db.manage_trail(&vault, None, serde_json::from_value(req).unwrap())
            .unwrap();
    }
    assert_eq!(
        scan(&mut db, None).findings[&reference].digest,
        Verdict::NotOnTrail
    );
    let archived = scan(&mut db, Some(&verify));
    assert_eq!(archived.archived, 1);
    assert!(archived.findings.is_empty());
    assert_eq!(
        db.verify_object_with_archive("app.Cell", object, Some(&verify))
            .unwrap()
            .digest,
        Verdict::Matches
    );
    let ciphertext = std::fs::read_dir(&archive_dir)
        .unwrap()
        .filter_map(Result::ok)
        .find(|f| f.file_name().len() == 64)
        .unwrap()
        .path();
    let digest = ciphertext.file_name().unwrap().to_str().unwrap();
    assert!(verify.read(digest).is_err());
    let original = std::fs::read(&ciphertext).unwrap();
    let mut corrupt = original.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    std::fs::write(&ciphertext, corrupt).unwrap();
    assert!(db.check_kind("app.Cell", Some(&verify)).is_err());
    std::fs::write(&ciphertext, original).unwrap();
    raw("DELETE FROM \"pd_app__Cell\" WHERE f4='external-edit'");
    assert_eq!(
        scan(&mut db, Some(&verify)).findings[&reference].digest,
        Verdict::Missing
    );
    assert!(
        db.transaction(&Scope::Internal, |tx| tx.record_objects(
            "app.Cell",
            &[object],
            "/ops/accept",
            false
        ))
        .is_err()
    );
    db.transaction(&Scope::Internal, |tx| {
        tx.record_objects_with_archive("app.Cell", &[object], "/ops/removed", true, Some(&verify))
    })
    .unwrap();
    assert!(scan(&mut db, Some(&verify)).findings.is_empty());
    db.seed(vec![(
        "app.Cell".into(),
        json!({"id":STANDARD.encode(object),"tenant":{"id":STANDARD.encode(t)},"alias":"restored"}),
    )])
    .unwrap();
    assert_eq!(
        scan(&mut db, Some(&verify)).findings[&reference].digest,
        Verdict::Differs
    );
    raw("UPDATE \"pd_app__Audit\" SET f21=substr(f21,1,0) WHERE f10='/ops/removed'");
    assert_eq!(
        scan(&mut db, Some(&verify)).findings[&reference].digest,
        Verdict::NoDigest
    );
    // The current row is not an input to the MAC of a carried line.
    let person = paydirt::mint(2);
    let pid = paydirt::format_id(&person).unwrap();
    db.seed(vec![("app.Holder".into(),json!({"id":STANDARD.encode(person),"tenant":{"id":STANDARD.encode(t)},"alias":"member","name":"First"}))]).unwrap();
    db.transaction(&Scope::Internal, |tx| {
        tx.record_objects("app.Holder", &[person], "/ops/initial", false)
    })
    .unwrap();
    let manage = |db: &mut Store, req: serde_json::Value| {
        db.manage_trail(&vault, None, serde_json::from_value(req).unwrap())
            .unwrap()
    };
    manage(
        &mut db,
        json!({"tenant":tenant,"action":"policy","policy":{"by":{"holder":{"retain_ms":0}}}}),
    );
    manage(&mut db, json!({"tenant":tenant,"action":"collect"}));
    raw("UPDATE \"pd_app__Holder\" SET f5='Edited' WHERE f4='member'");
    manage(
        &mut db,
        json!({"tenant":tenant,"action":"policy","policy":{"by":{"holder":{"retain_ms":0,"discard":true}}}}),
    );
    assert_eq!(
        manage(
            &mut db,
            json!({"tenant":tenant,"action":"collect","dry_run":true})
        )
        .carried,
        [pid.as_str()]
    );
    assert_eq!(
        manage(&mut db, json!({"tenant":tenant,"action":"collect"})).carried,
        [pid.as_str()]
    );
    let check = db.check_kind("app.Holder", Some(&verify)).unwrap();
    assert_eq!(check.archived, 0);
    assert_eq!(check.findings[pid.as_str()].seal, Some(Verdict::Differs));
    raw("UPDATE \"pd_app__Holder\" SET f5='First' WHERE f4='member'");
    assert!(
        db.check_kind("app.Holder", Some(&verify))
            .unwrap()
            .findings
            .is_empty()
    );
    let original_date = db
        .call(
            &Scope::Internal,
            Call {
                entity: "app.Holder".into(),
                method: "Get".into(),
                request: json!({"ref":{"id":STANDARD.encode(person)}}),
            },
        )
        .unwrap()["dateCreated"]
        .as_str()
        .unwrap()
        .to_owned();
    raw("UPDATE \"pd_app__Holder\" SET f15='2000-01-01T00:00:00Z' WHERE f4='member'");
    assert_eq!(
        db.verify_object("app.Holder", person).unwrap().digest,
        Verdict::Differs
    );
    raw(&format!(
        "UPDATE \"pd_app__Holder\" SET f15='{original_date}' WHERE f4='member'"
    ));
    if url.starts_with("postgres") {
        {
            let barrier = std::sync::Barrier::new(2);
            let run = |arg| {
                let mut store = open(6);
                let archive = Archive::open(&archive_dir, &[54; 32])
                    .unwrap()
                    .with_notary(notary(directory.join("witness")));
                barrier.wait();
                store
                    .manage_trail(&archive, None, serde_json::from_value(arg).unwrap())
                    .unwrap();
            };
            // Both stores target the same committed state on separate connections.
            let forget = json!({"tenant":tenant,"action":"forget","subject":pid});
            let collect = json!({"tenant":tenant,"action":"collect"});
            std::thread::scope(|threads| {
                threads.spawn(|| run(forget));
                threads.spawn(|| run(collect));
            });
        }
    } else {
        manage(
            &mut db,
            json!({"tenant":tenant,"action":"forget","subject":pid}),
        );
        manage(&mut db, json!({"tenant":tenant,"action":"collect"}));
    }
    assert_eq!(
        db.check_kind("app.Holder", Some(&verify))
            .unwrap()
            .forgotten,
        1
    );
    assert_eq!(
        db.verify_object("app.Holder", person).unwrap().seal,
        Some(Verdict::Forgotten)
    );
    db.transaction(&Scope::Internal, |tx| {
        tx.record_objects("app.Holder", &[person], "/ops/approve", false)
    })
    .unwrap();
    assert_eq!(
        db.verify_object("app.Holder", person).unwrap().seal,
        Some(Verdict::Matches)
    );
}

#[cfg(feature = "sqlite")]
#[test]
fn scans_current_and_signed_archived_rows_and_records_explicit_acceptance() {
    use diesel::{Connection, connection::SimpleConnection};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let url = path.to_str().unwrap();
    check_database(url, dir.path(), |sql| {
        diesel::SqliteConnection::establish(url)
            .unwrap()
            .batch_execute(sql)
            .unwrap()
    });
    #[cfg(feature = "postgres")]
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let admin = || diesel::PgConnection::establish(&url).unwrap();
        let schema = format!("integrity_{}", hex::encode(paydirt::mint(0)));
        admin()
            .batch_execute(&format!("CREATE SCHEMA {schema}"))
            .unwrap();
        let scoped = format!(
            "{url}{}options=-csearch_path%3D{schema}",
            if url.contains('?') { "&" } else { "?" }
        );
        let dir = tempfile::tempdir().unwrap();
        let result = std::panic::catch_unwind(|| {
            check_database(&scoped, dir.path(), |sql| {
                diesel::PgConnection::establish(&scoped)
                    .unwrap()
                    .batch_execute(sql)
                    .unwrap()
            })
        });
        admin()
            .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
            .unwrap();
        if let Err(err) = result {
            std::panic::resume_unwind(err)
        }
    }
}
