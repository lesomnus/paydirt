use paydirt_store::{Fault, Violation};

#[test]
fn nested_errors_keep_each_path_and_the_original_status() {
    let fault = Fault::validation([
        Violation::new("[1].alias", "invalid alias"),
        Violation::new("", "missing"),
    ])
    .at("items")
    .at("request");
    assert_eq!(fault.code, "invalid_argument");
    assert_eq!(
        fault.violations,
        [
            Violation::new("request.items[1].alias", "invalid alias"),
            Violation::new("request.items", "missing")
        ]
    );
    assert_eq!(
        fault.message,
        "request.items[1].alias: invalid alias; request.items: missing"
    );
    let refused = Fault::denied("no access");
    assert_eq!(refused.clone().at("tenant"), refused);
    assert_eq!(Fault::validation([]).code, "internal");
}

#[test]
fn legacy_faults_can_be_nested_after_a_worker_round_trip() {
    let fault: Fault =
        serde_json::from_str(r#"{"code":"invalid_argument","message":"required","field":"alias"}"#)
            .unwrap();
    assert_eq!(
        fault.at("tenant").violations,
        [Violation::new("tenant.alias", "required")]
    );
    assert_eq!(
        Fault::invalid("required").at("tenant").violations,
        [Violation::new("tenant", "required")]
    );
}
