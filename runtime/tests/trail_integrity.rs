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
