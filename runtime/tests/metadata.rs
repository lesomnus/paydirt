use paydirt_store::{
    domain::Domains,
    header::{Field, Header},
    paydirt_model::Model,
};
use serde_json::json;

#[test]
fn historical_and_qualified_names_survive_ambiguity() {
    let mut d = Domains::default();
    d.register("a.WorkCell", 9, "").unwrap();
    d.formerly("a.Arm", 9, "").unwrap();
    assert_eq!(d.lookup("a.Arm"), Some(9));
    assert_eq!(d.domain_of("arm"), Some(9));
    assert_eq!(d.name(9), "work-cell");
    d.register("b.Arm", 10, "").unwrap();
    assert_eq!(d.domain_of("arm"), Some(10));
    d.register("c.Arm", 11, "").unwrap();
    assert_eq!(d.domain_of("arm"), None);
    assert_eq!(d.lookup("c.Arm"), Some(11));
    d.register("b.Site", 9, "").unwrap();
    assert_eq!(d.name(9), "domain(9)");
    assert!(d.entries().all(|(id, _)| id != 9));
    assert!(d.register("b.Site", 12, "").is_err());
}
#[test]
fn header_distinguishes_declared_empty_and_absent_fields() {
    let fields = [
        ("id", "UUID", ""),
        ("alias", "STRING", ""),
        ("name", "STRING", ""),
        ("labels", "JSON", "map"),
        ("date_created", "TIME", ""),
        ("tenant_id", "BYTES", ""),
    ];
    let fields:Vec<_>=fields.iter().enumerate().map(|(i,(name,kind,collection))|json!({"name":name,"json":name,"number":i+1,"kind":kind,"collection":collection})).collect();
    let m:Model=serde_json::from_value(json!({"version":1,"entities":[{"name":"test.Sample","key":"id","fields":fields,"indexes":[]}]})).unwrap();
    let e = &m.entities[0];
    let h = Header::read(
        &m,
        e,
        &json!({"alias":"empty","id":"AQEBAQEBAQEBAQEBAQEBAQ=="}),
    )
    .unwrap();
    assert_eq!(h.id, [1; 16]);
    assert!(h.has(Field::Name));
    assert!(h.has(Field::Created));
    assert!(h.created.is_none());
    assert!(h.has(Field::Tenant));
    assert!(!h.has(Field::Desc));
    assert_eq!(h.title(), "empty");
    assert_eq!(
        Header::read(&m, e, &json!({"id":"AQEBAQEBAQEBAQEBAQEBAQ=="}))
            .unwrap()
            .title(),
        "01010101-0101-0101-0101-010101010101"
    );
    assert_eq!(
        Header::read(&m, e, &serde_json::Value::Null).unwrap(),
        Header::default()
    );
}
