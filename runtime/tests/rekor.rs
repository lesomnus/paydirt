#![cfg(not(target_arch = "wasm32"))]
use paydirt_store::{
    attestation::{Blob, DirectoryWitness, Ed25519, LogWitness, Notary, Witnesses},
    rekor::Rekor,
};
use std::{collections::BTreeSet, fs, path::PathBuf};

#[test]
#[ignore = "requires the pinned Go Rekor test servers via scripts/test-rekor.py"]
fn original_rekor_server() {
    let root = PathBuf::from(std::env::var_os("PAYDIRT_TEST_ROOT").unwrap());
    let spec = std::env::var("PAYDIRT_TEST_REKOR").unwrap();
    let notary = Notary::new(
        Box::new(Ed25519::from_pkcs8(&fs::read(root.join("key.der")).unwrap()).unwrap()),
        Default::default(),
        Box::new(Rekor::open(&spec).unwrap()),
    );
    let members = |count| {
        (1..=count)
            .map(|n| Blob::of("audit", &format!("{n:064x}")))
            .collect::<BTreeSet<_>>()
    };
    let check = |n, advance| notary.check_chain(&root, members(n), Default::default(), advance);
    let dir = root
        .join(".receipts")
        .read_dir()
        .ok()
        .and_then(|mut paths| paths.next())
        .map(|e| e.unwrap().path());
    match std::env::var("PAYDIRT_TEST_OPERATION").as_deref().unwrap() {
        "publish" => {
            for i in 1..=5 {
                assert_eq!(check(i, true).unwrap().unwrap().k, i);
            }
        }
        "verify" => {
            assert_eq!(check(5, false).unwrap().unwrap().k, 5);
        }
        "tamper" => {
            let receipt = dir.unwrap().join("00000000000000000003.json");
            let previous = fs::read(&receipt).unwrap();
            let mut damaged: serde_json::Value = serde_json::from_slice(&previous).unwrap();
            let proof = if spec.ends_with("v=2") {
                &mut damaged["inclusionProof"]
            } else {
                &mut damaged["verification"]["inclusionProof"]
            };
            proof["hashes"][0] = "damaged".into();
            fs::write(&receipt, serde_json::to_vec(&damaged).unwrap()).unwrap();
            assert!(check(5, false).is_err());
            fs::write(&receipt, &previous).unwrap();
            assert!(check(5, false).is_ok());
        }
        "truncate" => {
            let gz = root.join(".checkpoints/00000000000000000005.gz");
            let receipt = dir.unwrap().join("00000000000000000005.json");
            let files = [gz, receipt].map(|p| {
                let bytes = fs::read(&p).unwrap();
                fs::remove_file(&p).unwrap();
                (p, bytes)
            });
            assert!(check(4, false).unwrap_err().message.contains("unkept"));
            for (p, bytes) in files {
                fs::write(p, bytes).unwrap();
            }
        }
        "unavailable" | "forked" => {
            assert!(check(5, false).is_err());
        }
        operation => panic!("unknown operation {operation}"),
    }
}

#[test]
fn directory_and_log_witnesses_detect_and_refuse_a_rewound_checkpoint() {
    let root = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let witnesses = Witnesses(vec![
        Box::new(LogWitness),
        Box::new(DirectoryWitness(external.path().to_owned())),
    ]);
    let n = Notary::new(
        Box::new(Ed25519::from_seed(&[7; 32]).unwrap()),
        Default::default(),
        Box::new(witnesses),
    );
    let members = BTreeSet::from([Blob::of("events", &"ab".repeat(32))]);
    let check = |advance| n.check_chain(root.path(), members.clone(), Default::default(), advance);
    for k in [1, 2] {
        assert_eq!(check(true).unwrap().unwrap().k, k);
    }
    assert_eq!(check(false).unwrap().unwrap().k, 2);
    for path in [
        ".checkpoints/00000000000000000002.gz",
        ".receipts/log/00000000000000000002.json",
    ] {
        fs::remove_file(root.path().join(path)).unwrap();
    }
    assert!(
        check(false)
            .unwrap_err()
            .message
            .contains("external witness disagrees")
    );
    assert!(check(true).is_err());
    assert!(
        !root
            .path()
            .join(".checkpoints/00000000000000000002.gz")
            .exists()
    );
}
