#![cfg(feature = "sqlite")]
use paydirt_store::{Fault, Store};
use serde_json::{Value, json};
#[test]
fn a_failed_meter_leaves_other_readings_deliverable() {
    let mut policies: Value =
        serde_json::from_str(include_str!("../../generated/storage/policies.json")).unwrap();
    policies["app.Holder"]["application"] = json!({"meters":[{"key":"fleet.seats","unit":"{seat}","count":{"of":"Holder"}},{"key":"fleet.invalid","unit":"1","count":{"of":"Missing"}}]});
    let mut db = Store::open(
        ":memory:",
        include_str!("../../generated/storage/model.json"),
        &policies.to_string(),
    )
    .unwrap();
    let batch = db.measure(&paydirt::mint(1)).unwrap();
    assert_eq!(batch.readings.len(), 1);
    assert_eq!(batch.errors.len(), 1);
    let mut sent = false;
    let error = batch
        .deliver(|s| {
            sent = true;
            assert_eq!(s[0].value, 0);
            Err(Fault::internal("sink failed"))
        })
        .unwrap_err();
    assert!(
        sent && error.message.contains("fleet.invalid") && error.message.contains("sink failed")
    );
    assert!(!paydirt_store::meter::valid_key("counter"));
    assert!(!paydirt_store::meter::valid_unit("two words"));
}

#[test]
fn aligned_pass_retries_only_unsettled_measurements() {
    use chrono::{DateTime, Utc};
    use paydirt_store::meter::{Measurements, Pass, Reading, next_boundary};
    use std::time::Duration;
    let now = DateTime::parse_from_rfc3339("2025-03-19T17:32:10Z")
        .unwrap()
        .with_timezone(&Utc);
    for (secs, expected) in [
        (3600, "2025-03-19 18:00:00 UTC"),
        (86400, "2025-03-20 00:00:00 UTC"),
        (604800, "2025-03-20 00:00:00 UTC"),
    ] {
        assert_eq!(
            next_boundary(now, Duration::from_secs(secs))
                .unwrap()
                .to_string(),
            expected
        );
    }
    let mut pass = Pass::new(now);
    let mut reads = [0; 2];
    let mut deliveries = Vec::new();
    let mut fail = true;
    for attempt in 0..2 {
        let errors = pass.run_with(
            &mut reads,
            |_, _, _| Ok(vec![vec![0], vec![1]]),
            |reads, id, at| {
                let n = id[0] as usize;
                reads[n] += 1;
                let row = |name: &str| Reading {
                    tenant: json!(n),
                    meter: name.into(),
                    measured_at: at.to_string(),
                    value: reads[n],
                    unit: "1".into(),
                };
                let mut readings = vec![row("test.a")];
                let errors = if n == 1 && reads[n] == 1 {
                    vec!["failed b".into()]
                } else {
                    readings.push(row("test.b"));
                    vec![]
                };
                Ok(Measurements { readings, errors })
            },
            |rows| {
                if rows[0].tenant == 0 && fail {
                    fail = false;
                    return Err(Fault::unavailable("offline"));
                }
                deliveries.extend(rows.iter().map(|r| {
                    (
                        r.tenant.as_u64().unwrap(),
                        r.meter.clone(),
                        r.value,
                        r.measured_at.clone(),
                    )
                }));
                Ok(())
            },
        );
        assert_eq!(errors.is_empty(), attempt == 1);
        assert_eq!(pass.done(), attempt == 1);
    }
    assert_eq!(reads, [1, 2]);
    assert_eq!(deliveries.len(), 4);
    assert!(deliveries.iter().all(|r| r.3 == now.to_string()));
    assert_eq!(
        deliveries
            .iter()
            .find(|r| r.0 == 1 && r.1 == "test.a")
            .unwrap()
            .2,
        1
    );
    assert_eq!(
        deliveries
            .iter()
            .find(|r| r.0 == 1 && r.1 == "test.b")
            .unwrap()
            .2,
        2
    );
}

#[test]
fn tenant_pages_resume_after_the_last_id() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use paydirt_store::{Call, Scope};
    let mut db = Store::open(
        ":memory:",
        include_str!("../../generated/storage/model.json"),
        include_str!("../../generated/storage/policies.json"),
    )
    .unwrap();
    let mut ids = Vec::new();
    for _ in 0..3 {
        let t = db
            .call(
                &Scope::Internal,
                Call {
                    entity: "app.Tenant".into(),
                    method: "Add".into(),
                    request: json!({}),
                },
            )
            .unwrap();
        ids.push(STANDARD.decode(t["id"].as_str().unwrap()).unwrap());
    }
    ids.sort();
    assert_eq!(db.meter_tenants(None, 100).unwrap(), ids);
    let mut walked = Vec::new();
    loop {
        let page = db
            .meter_tenants(walked.last().map(Vec::as_slice), 1)
            .unwrap();
        if page.is_empty() {
            break;
        }
        assert_eq!(page.len(), 1);
        walked.extend(page);
    }
    assert_eq!(walked, ids);
    assert_eq!(db.meter_tenants(None, 2).unwrap(), ids[..2]);
    assert_eq!(db.meter_tenants(Some(&ids[1]), 2).unwrap(), ids[2..]);
    assert!(db.meter_tenants(Some(&ids[2]), 1).unwrap().is_empty());
}

#[cfg(feature = "workers")]
#[test]
fn retrying_local_sink_records_each_measurement_once() {
    use paydirt_store::meter::{Reading, directory_sink};
    let tmp = tempfile::tempdir().unwrap();
    let row = Reading {
        tenant: json!("A"),
        meter: "x.y".into(),
        measured_at: "2025-03-19T00:00:00Z".into(),
        value: 3,
        unit: "1".into(),
    };
    for _ in 0..2 {
        directory_sink(tmp.path(), std::slice::from_ref(&row)).unwrap();
    }
    let files = std::fs::read_dir(tmp.path())
        .unwrap()
        .collect::<std::io::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(files.len(), 1);
    let read: Reading = serde_json::from_slice(&std::fs::read(files[0].path()).unwrap()).unwrap();
    assert_eq!(read.value, 3);
}
