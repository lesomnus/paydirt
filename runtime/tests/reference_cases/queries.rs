use super::walls::{call, frame, invoke, key, owner, world};
use super::*;

#[test]
fn declared_headers_keep_identity_title_tenant_and_presence_across_managed_types() {
    use paydirt_store::header::{Field, Header};
    backends(|f, url, _| {
        let mut db = f.open(url, &f.policies, false);
        let t = provision(&mut db);
        let tenant = db
            .operator(call("Tenant", "Get", json!({"ref":owner(t)})))
            .unwrap();
        let holder = db
            .operator(call(
                "Holder",
                "Add",
                json!({"tenant":owner(t),"alias":"admin"}),
            ))
            .unwrap();
        let arm = db
            .operator(call(
                "Robot",
                "Add",
                json!({"tenant":owner(t),"alias":"arm-01"}),
            ))
            .unwrap();
        let audit = table_rows(&mut db, "app.Audit")
            .into_iter()
            .find(|r| r["objectId"] == arm["id"])
            .unwrap();
        let model: paydirt_model::Model = serde_json::from_str(&f.model).unwrap();
        let head = |kind: &str, row: &Value| {
            Header::read(&model, model.entity(&format!("app.{kind}")).unwrap(), row).unwrap()
        };
        let h = head("Robot", &arm);
        assert_eq!(h.id, id(&arm["id"]));
        assert_eq!(h.title(), "arm-01");
        assert_eq!(h.tenant, t);
        assert_eq!(
            h.id[9],
            f.policies["app.Robot"]["entity"]["domain"]
                .as_u64()
                .unwrap() as u8
        );
        let h = head("Tenant", &tenant);
        assert_eq!(h.alias, "owner");
        assert!(h.has(Field::Labels));
        assert!(!h.has(Field::Tenant));
        let h = head("Holder", &holder);
        assert_eq!(h.alias, "admin");
        assert!(h.has(Field::Tenant) && h.has(Field::Erased));
        assert_eq!(h.tenant, t);
        let h = head("Audit", &audit);
        assert!(h.has(Field::Id) && h.has(Field::Tenant));
        assert!(!h.has(Field::Alias) && !h.has(Field::Name));
        assert_eq!(h.tenant, t);
        assert_eq!(h.title(), paydirt::format_id(&h.id).unwrap());
    });
}

#[test]
fn lists_apply_scope_and_declared_filters_before_paging_and_expand_declared_edges() {
    use std::collections::BTreeSet;
    backends(|f, url, _| {
        let mut db = f.open(url, &f.policies, false);
        let (a, b) = world(&mut db);
        let who = frame(a, paydirt::mint(2), &[a], "");
        let mut own = vec![];
        for (t, n, stem) in [(b, 20, "away"), (a, 2, "local")] {
            for i in 0..n {
                let r = db
                    .operator(call(
                        "Robot",
                        "Add",
                        json!({"tenant":owner(t),"alias":format!("{stem}-{i:02}")}),
                    ))
                    .unwrap();
                if t == a {
                    own.push(r);
                }
            }
        }
        let list = |db: &mut Store, q| invoke(db, &who, call("Robot", "List", q));
        let page = list(&mut db, json!({"size":5})).unwrap();
        assert_eq!(page["items"].as_array().unwrap().len(), 2);
        assert_eq!(page["items"][0]["id"], own[0]["id"]);
        assert!(page["next"].as_str().unwrap_or("").is_empty());
        assert_eq!(page["items"][0]["tenant"]["id"], STANDARD.encode(a));
        own.push(
            db.operator(call(
                "Robot",
                "Add",
                json!({"tenant":owner(a),"alias":"local-02"}),
            ))
            .unwrap(),
        );
        assert_eq!(
            list(&mut db, json!({"size":1000})).unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            list(
                &mut db,
                json!({"filters":vec![json!({"ref":key(&own[0])});33]})
            )
            .unwrap_err()
            .code,
            "invalid_argument"
        );
        let selected = list(
            &mut db,
            json!({"filters":[{"ref":key(&own[0])},{"ref":key(&own[2])}]}),
        )
        .unwrap();
        assert_eq!(
            selected["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["id"].clone())
                .collect::<Vec<_>>(),
            vec![own[0]["id"].clone(), own[2]["id"].clone()]
        );
        for q in [json!({"filters":[{}]}), json!({"order":999})] {
            assert_eq!(list(&mut db, q).unwrap_err().code, "invalid_argument");
        }
        for (tenant, expected) in [(owner(a), 3), (json!({"alias":"another"}), 20)] {
            let results = db
                .operator(call(
                    "Robot",
                    "List",
                    json!({"filters":[{"tenant":tenant}]}),
                ))
                .unwrap();
            assert_eq!(results["items"].as_array().unwrap().len(), expected);
        }
        let mut names = BTreeSet::new();
        for i in 0..7 {
            db.operator(call(
                "Holder",
                "Add",
                json!({"tenant":owner(a),"alias":format!("op-{i:03}")}),
            ))
            .unwrap();
        }
        let mut after = String::new();
        for _ in 0..4 {
            let v = invoke(
                &mut db,
                &who,
                call("Holder", "List", json!({"size":3,"after":after})),
            )
            .unwrap();
            for row in v["items"].as_array().unwrap() {
                assert!(names.insert(row["alias"].as_str().unwrap().to_owned()));
            }
            let next = v["next"].as_str().unwrap_or("");
            assert_ne!(after, next);
            if next.is_empty() {
                break;
            }
            after = next.into();
        }
        assert_eq!(names.len(), 7);
    });
}

#[test]
fn alias_rules_cover_generated_names_exhaustion_existing_rows_and_deployment_writes() {
    use paydirt_store::naming;
    use std::{
        collections::BTreeSet,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    backends(|f, url, _| {
        let mut db = f.open(url, &f.policies, false);
        let t = provision(&mut db);
        let who = frame(t, paydirt::mint(2), &[t], "");
        let add = |alias: &str| call("Robot", "Add", json!({"tenant":owner(t),"alias":alias}));
        for alias in ["Not An Alias!!", "-leading", "trailing-", "9front", "a--b"] {
            let err = invoke(&mut db, &who, add(alias)).unwrap_err();
            assert_eq!(err.code, "invalid_argument");
            assert_eq!(err.field.as_deref(), Some("alias"));
        }
        assert_eq!(
            db.operator(call("Tenant", "Add", json!({"alias":"Ops Team"})))
                .unwrap_err()
                .field
                .as_deref(),
            Some("alias")
        );
        assert_eq!(
            db.operator(call("Tenant", "Add", json!({"alias":" OPS "})))
                .unwrap()["alias"],
            "ops"
        );
        let arm = invoke(&mut db, &who, add("  Arm-01  ")).unwrap();
        let slug = json!({"slug":{"tenant":{"alias":"OWNER"},"alias":"ARM-01"}});
        assert_eq!(
            invoke(&mut db, &who, call("Robot", "Get", json!({"ref":slug}))).unwrap()["id"],
            arm["id"]
        );
        let mut seen = BTreeSet::new();
        for _ in 0..8 {
            let row = invoke(&mut db, &who, add("")).unwrap();
            let n = row["alias"].as_str().unwrap();
            assert!(paydirt::rbac::alias(n));
            assert!(seen.insert(n.to_owned()));
        }
        let mut db = db.with_namer(naming::required);
        for (kind, req) in [("Tenant", json!({})), ("Robot", json!({"tenant":owner(t)}))] {
            assert_eq!(
                db.operator(call(kind, "Add", req))
                    .unwrap_err()
                    .field
                    .as_deref(),
                Some("alias")
            );
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let mut db = db.with_namer(move |_: naming::NameRequest<'_>| {
            captured.fetch_add(1, Ordering::SeqCst);
            Ok("only-one".into())
        });
        invoke(&mut db, &who, add("")).unwrap();
        assert_eq!(
            invoke(&mut db, &who, add("")).unwrap_err().code,
            "already_exists"
        );
        assert!(calls.load(Ordering::SeqCst) >= 3);
        let before = calls.load(Ordering::SeqCst);
        assert_eq!(
            invoke(
                &mut db,
                &who,
                call(
                    "Robot",
                    "Patch",
                    json!({"ref":key(&arm),"alias":"","dateUpdatedForce":true})
                )
            )
            .unwrap_err()
            .code,
            "invalid_argument"
        );
        assert_eq!(calls.load(Ordering::SeqCst), before);
    });
}

#[test]
fn message_columns_round_trip_replace_and_expose_protobuf_json_names_in_sql() {
    backends(|f, url, _| {
        let mut db = f.open(url, &f.policies, false);
        let t = provision(&mut db);
        let profile = json!({"displayName":"Ada Lovelace","howMany":3});
        let r = db
            .operator(call(
                "Holder",
                "Add",
                json!({"tenant":owner(t),"alias":"ada","profile":profile}),
            ))
            .unwrap();
        let get = || call("Holder", "Get", json!({"ref":key(&r)}));
        assert_eq!(db.operator(get()).unwrap()["profile"], profile);
        let model: paydirt_model::Model = serde_json::from_str(&f.model).unwrap();
        let col = model
            .entity("app.Holder")
            .unwrap()
            .field("profile")
            .unwrap()
            .column();
        for (key, value, n) in [
            ("displayName", "Ada Lovelace", 1),
            ("howMany", "3", 1),
            ("how_many", "3", 0),
        ] {
            let path = if url.starts_with("postgres") {
                format!("({col}::jsonb ->> '{key}')")
            } else {
                format!("CAST(json_extract({col},'$.{key}') AS TEXT)")
            };
            assert_eq!(
                count_sql(
                    url,
                    &format!(
                        r#"SELECT count(*) AS n FROM "pd_app__Holder" WHERE {path}='{value}'"#
                    )
                ),
                n
            );
        }
        db.operator(call(
            "Holder",
            "Patch",
            json!({"ref":key(&r),"dateUpdatedForce":true,"profile":{"displayName":"A. Lovelace"}}),
        ))
        .unwrap();
        assert_eq!(
            db.operator(get()).unwrap()["profile"],
            json!({"displayName":"A. Lovelace"})
        );
    });
}

#[test]
fn a_required_column_from_an_earlier_schema_reports_an_executable_repair() {
    backends(|f, url, raw| {
        let db = f.open(url, &f.policies, false);
        drop(db);
        let table = r#""pd_app__Event""#;
        raw(&format!(
            "ALTER TABLE {table} ADD COLUMN description TEXT NOT NULL"
        ));
        let open = || Store::open(url, &f.model, &f.policies.to_string());
        let e = open().err().expect("required unknown column must fail");
        assert_eq!(e.code, "failed_precondition");
        assert!(e.message.contains("pd_app__Event.description"));
        let fixes = e
            .message
            .lines()
            .filter(|l| l.starts_with("ALTER TABLE "))
            .collect::<Vec<_>>();
        assert_eq!(fixes.len(), 1);
        raw(fixes[0]);
        let mut restored = open().unwrap();
        let t = provision(&mut restored);
        assert_eq!(
            restored
                .operator(call("Tenant", "Get", json!({"ref":owner(t)})))
                .unwrap()["id"],
            STANDARD.encode(t)
        );
    });
}
