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
