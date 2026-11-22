use paydirt::rbac::{Binding, Group, Policy, Role, Rules, Snapshot};

fn rules() -> Rules {
    Rules {
        roles: [(
            "@acme/reader".into(),
            Role {
                methods: vec!["/app.*/Get".into()],
            },
        )]
        .into(),
        groups: [(
            "@acme/readers".into(),
            Group {
                members: vec!["@acme/alice".into(), "@acme/alice".into()],
            },
        )]
        .into(),
        bindings: vec![Binding {
            role: "@acme/reader".into(),
            group: Some("@acme/readers".into()),
            holder: None,
        }],
    }
}
#[test]
fn bindings_stay_in_one_tenant_and_are_pinned_per_call() {
    let store = Policy::new(Snapshot::new(rules()).unwrap());
    let (version, held) = store.pin();
    assert!(held.allows("@acme/alice", "/app.RobotService/Get"));
    assert!(!held.allows("@other/alice", "/app.RobotService/Get"));
    assert!(!held.allows("@acme/alice", "/app.RobotService/Add"));
    assert_eq!(held.holders("@acme/reader").unwrap(), ["@acme/alice"]);
    store.replace(Snapshot::new(rules()).unwrap());
    assert_eq!(store.pin().0, version);
    store.replace(Snapshot::default());
    assert_ne!(store.pin().0, version);
    assert!(held.allows("@acme/alice", "/app.CellService/Get"));
    assert!(!store.pin().1.allows("@acme/alice", "/app.CellService/Get"));
}
#[test]
fn broken_rules_reject_the_whole_snapshot() {
    let mut r = rules();
    r.groups
        .values_mut()
        .next()
        .unwrap()
        .members
        .push("@foreign/guest".into());
    assert!(Snapshot::new(r).is_err());
    let mut r = rules();
    r.bindings[0].holder = Some("@acme/alice".into());
    assert!(Snapshot::new(r).is_err());
    let mut r = rules();
    r.roles
        .values_mut()
        .next()
        .unwrap()
        .methods
        .push("/app.Robot*/Get".into());
    assert!(Snapshot::new(r).is_err());
    let mut r = rules();
    r.groups.clear();
    assert!(Snapshot::new(r).is_err());
}
