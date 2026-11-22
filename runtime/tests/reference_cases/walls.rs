use super::*;

pub(super) fn frame(owner: [u8; 16], actor: [u8; 16], tenants: &[[u8; 16]], action: &str) -> Frame {
    serde_json::from_value(json!({"tenant":owner,"actor":actor,"tenants":tenants,"action":action}))
        .unwrap()
}
pub(super) fn call(kind: &str, method: &str, request: Value) -> Call {
    Call {
        entity: format!("app.{kind}"),
        method: method.into(),
        request,
    }
}
pub(super) fn invoke(
    store: &mut Store,
    who: &Frame,
    operation: Call,
) -> paydirt_store::Result<Value> {
    let mut f = who.clone();
    if f.action.is_empty() {
        f.action = format!("/{}Service/{}", operation.entity, operation.method);
    }
    store.handle(Packet::BeginOrigin {
        id: "request".into(),
        frame: f,
        origin: operation.clone(),
    })?;
    let answer = store.handle(Packet::Call {
        id: "request".into(),
        call: operation,
    });
    store.handle(Packet::Finish {
        id: "request".into(),
        commit: answer.is_ok(),
    })?;
    answer
}
pub(super) fn world(db: &mut Store) -> ([u8; 16], [u8; 16]) {
    let a = provision(db);
    let b = paydirt::mint(1);
    db.seed(vec![(
        "app.Tenant".into(),
        json!({"id":STANDARD.encode(b),"alias":"another"}),
    )])
    .unwrap();
    (a, b)
}
pub(super) fn key(row: &Value) -> Value {
    json!({"id":row["id"]})
}
pub(super) fn owner(id: [u8; 16]) -> Value {
    json!({"id":STANDARD.encode(id)})
}

#[test]
fn trail_filing_keeps_source_rows_soft_erasure_and_a_second_partys_visibility() {
    backends(|f, url, _| {
        let mut db = access_store(f, url);
        let (a, b) = world(&mut db);
        let actor = paydirt::mint(2);
        let mine = frame(a, actor, &[a], "");
        let theirs = frame(b, actor, &[b], "");
        let both = frame(b, actor, &[a, b], "");
        let make = |db: &mut Store, who: &Frame, alias: &str| {
            invoke(
                db,
                who,
                call("Robot", "Add", json!({"tenant":owner(a),"alias":alias})),
            )
            .unwrap()
        };
        let row = make(&mut db, &mine, "arm-01");
        let joint = invoke(
            &mut db,
            &mine,
            call("Joint", "Add", json!({"robot":key(&row),"alias":"elbow"})),
        )
        .unwrap();
        let reading = invoke(
            &mut db,
            &mine,
            call("Reading", "Add", json!({"robot":key(&row),"celsius":15})),
        )
        .unwrap();
        let by_object = |db: &mut Store, who: &Frame, object: &Value| {
            invoke(
                db,
                who,
                call(
                    "Audit",
                    "List",
                    json!({"filters":[{"objectId":object}],"size":200}),
                ),
            )
            .unwrap()["items"]
                .as_array()
                .unwrap()
                .clone()
        };
        assert!(by_object(&mut db, &theirs, &row["id"]).is_empty());
        let mut concerned = mine.clone();
        concerned.counterpart = Some(b.to_vec());
        invoke(
            &mut db,
            &concerned,
            call(
                "Robot",
                "Patch",
                json!({"ref":key(&row),"dateUpdatedForce":true,"name":"renamed"}),
            ),
        )
        .unwrap();
        let visible = by_object(&mut db, &theirs, &row["id"]);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0]["tenantId"], STANDARD.encode(a));
        assert_eq!(visible[0]["counterpartTenantId"], STANDARD.encode(b));
        let ours = by_object(&mut db, &mine, &row["id"]);
        assert!(
            ours.iter()
                .find(|r| r["action"] == "/app.RobotService/Add")
                .unwrap()["counterpartTenantId"]
                .is_null()
        );
        for field in ["tenantId", "actorTenantId", "objectId"] {
            let v = if field == "objectId" {
                row["id"].clone()
            } else {
                json!(STANDARD.encode(a))
            };
            assert!(
                !invoke(
                    &mut db,
                    &mine,
                    call("Audit", "List", json!({"filters":[{field:v}]}))
                )
                .unwrap()["items"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        invoke(&mut db, &both, call("Reading", "Erase", key(&reading))).unwrap();
        let r = by_object(&mut db, &theirs, &reading["id"])
            .into_iter()
            .find(|r| r["action"] == "/app.ReadingService/Erase")
            .unwrap();
        assert_eq!(r["tenantId"], STANDARD.encode(b));
        assert_eq!(r["value"], "");
        assert_eq!(r["domain"], f.policies["app.Reading"]["entity"]["domain"]);
        assert_eq!(id(&r["objectId"])[9], r["domain"].as_u64().unwrap() as u8);
        invoke(&mut db, &both, call("Robot", "Erase", key(&row))).unwrap();
        let erased = by_object(&mut db, &mine, &row["id"])
            .into_iter()
            .find(|r| r["action"] == "/app.RobotService/Erase")
            .unwrap();
        assert_eq!(erased["tenantId"], STANDARD.encode(a));
        assert_eq!(erased["actorTenantId"], STANDARD.encode(b));
        assert_eq!(erased["actorId"], STANDARD.encode(actor));
        let post = message_json(f, "app.Robot", &erased["value"]);
        assert_eq!(post["alias"], "arm-01");
        assert!(post["dateErased"].is_string());
        assert!(
            invoke(&mut db, &mine, call("Robot", "List", json!({}))).unwrap()["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            invoke(
                &mut db,
                &mine,
                call("Robot", "Get", json!({"ref":key(&row)}))
            )
            .unwrap_err()
            .code,
            "not_found"
        );
        assert_eq!(
            count_sql(
                url,
                r#"SELECT count(*) AS n FROM "pd_app__Robot" WHERE f4='arm-01' AND f14 IS NOT NULL"#
            ),
            1
        );
        invoke(
            &mut db,
            &both,
            call(
                "Joint",
                "Patch",
                json!({"ref":key(&joint),"alias":"shoulder","dateUpdatedForce":true}),
            ),
        )
        .unwrap();
        let changed = by_object(&mut db, &mine, &joint["id"])
            .into_iter()
            .find(|r| r["action"] == "/app.JointService/Patch")
            .unwrap();
        assert_eq!(changed["tenantId"], STANDARD.encode(a));
        assert_eq!(changed["actorTenantId"], STANDARD.encode(b));
        let post = message_json(f, "app.Joint", &changed["value"]);
        assert_eq!(post["alias"], "shoulder");
        assert!(post["dateErased"].is_null());
        assert_eq!(
            db.call(
                &Scope::Internal,
                call("Joint", "Get", json!({"ref":key(&joint)}))
            )
            .unwrap()["alias"],
            "shoulder"
        );
        let global = db
            .operator_as(
                call("Joint", "Add", json!({"alias":"orphan"})),
                actor,
                a,
                "shell:host".into(),
            )
            .unwrap();
        assert!(global["robot"].is_null());
        assert!(
            !table_rows(&mut db, "app.Audit")
                .iter()
                .filter(|r| r["objectId"] == global["id"])
                .collect::<Vec<_>>()
                .is_empty()
        );
        assert_eq!(
            invoke(
                &mut db,
                &mine,
                call("Joint", "Get", json!({"ref":key(&global)}))
            )
            .unwrap_err()
            .code,
            "not_found"
        );
    });
}

#[test]
fn managed_methods_and_host_scopes_compose_without_widening_tenant_reads() {
    use std::sync::Arc;
    backends(|f, url, _| {
        let mut db = f.open(url, &f.policies, false);
        let (a, b) = world(&mut db);
        let actor = paydirt::mint(2);
        let mine = frame(a, actor, &[a], "");
        let all = frame(a, actor, &[a, b], "");
        for (kind, method, req) in [
            ("Tenant", "Add", json!({"alias":"hidden"})),
            ("Tenant", "Erase", owner(a)),
            ("Audit", "Patch", json!({"ref":owner(a)})),
            ("Audit", "Erase", owner(a)),
        ] {
            assert_eq!(
                invoke(&mut db, &mine, call(kind, method, req))
                    .unwrap_err()
                    .code,
                "unimplemented"
            );
        }
        assert_eq!(
            invoke(
                &mut db,
                &mine,
                call("Holder", "Add", json!({"tenant":owner(b),"alias":"bob"}))
            )
            .unwrap_err()
            .code,
            "not_found"
        );
        let bob = invoke(
            &mut db,
            &all,
            call("Holder", "Add", json!({"tenant":owner(b),"alias":"bob"})),
        )
        .unwrap();
        let visible = invoke(&mut db, &mine, call("Tenant", "List", json!({}))).unwrap();
        assert_eq!(visible["items"].as_array().unwrap().len(), 1);
        assert_eq!(visible["items"][0]["id"], STANDARD.encode(a));
        assert_eq!(table_rows(&mut db, "app.Tenant").len(), 2);
        for (who, count) in [(&mine, 0), (&all, 1)] {
            let v = invoke(
                &mut db,
                who,
                call("Holder", "List", json!({"filters":[{"tenant":owner(b)}]})),
            )
            .unwrap();
            assert_eq!(v["items"].as_array().unwrap().len(), count);
            if count == 1 {
                assert_eq!(v["items"][0]["id"], bob["id"]);
            }
        }
        let mut keep = Value::Null;
        let mut excluded = Value::Null;
        let mut foreign = Value::Null;
        for (t, alias, target) in [
            (a, "keep", &mut keep),
            (a, "drop", &mut excluded),
            (b, "keep", &mut foreign),
        ] {
            *target = invoke(
                &mut db,
                &all,
                call("Robot", "Add", json!({"tenant":owner(t),"alias":alias})),
            )
            .unwrap();
        }
        let rule: paydirt_store::ScopeRule = Arc::new(|name, _| {
            Ok(if name == "app.Robot" {
                vec![("alias".into(), json!("keep"))]
            } else {
                vec![]
            })
        });
        let mut db = f
            .open(url, &f.policies, false)
            .with_scopes(vec![rule.clone()])
            .unwrap();
        assert_eq!(
            invoke(
                &mut db,
                &mine,
                call("Robot", "Get", json!({"ref":key(&keep)}))
            )
            .unwrap()["alias"],
            "keep"
        );
        for row in [&excluded, &foreign] {
            assert_eq!(
                invoke(
                    &mut db,
                    &mine,
                    call("Robot", "Get", json!({"ref":key(row)}))
                )
                .unwrap_err()
                .code,
                "not_found"
            );
        }
        assert_eq!(
            invoke(&mut db, &mine, call("Robot", "List", json!({}))).unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(
            db.with_scopes(vec![rule])
                .err()
                .unwrap()
                .message
                .contains("together")
        );
    });
}

#[test]
fn account_frames_see_declared_memberships_and_cannot_establish_an_account() {
    use paydirt_store::{AccountReads, ReadFilter, access::Observation};
    backends(|f, url, _| {
        let mut db = access_store(f, url);
        let (a, b) = world(&mut db);
        let both = paydirt::mint(40);
        let one = paydirt::mint(40);
        let actor = paydirt::mint(2);
        let make = |db: &mut Store, t, alias: &str, account: Option<[u8; 16]>| {
            let mut r = json!({"tenant":owner(t),"alias":alias});
            if let Some(ac) = account {
                r["account"] = json!(STANDARD.encode(ac));
            }
            db.call(&Scope::Internal, call("Holder", "Add", r)).unwrap()
        };
        let aa = make(&mut db, a, "alice", Some(both));
        let bb = make(&mut db, b, "bob", Some(both));
        let cc = make(&mut db, a, "other", Some(one));
        let lonely = make(&mut db, b, "no-account", None);
        assert!(db.memberships_for([0; 16], None, "").unwrap().is_empty());
        let ms = db.memberships_for(both, None, "").unwrap();
        assert_eq!(
            ms.iter().map(|r| &r["id"]).collect::<Vec<_>>(),
            [&aa["id"], &bb["id"]]
        );
        for selector in ["@another".to_owned(), paydirt::format_id(&b).unwrap()] {
            assert_eq!(
                db.memberships_for(both, None, &selector).unwrap()[0]["id"],
                bb["id"]
            );
            assert!(
                db.memberships_for(both, Some(a), &selector)
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(db.holder_account(id(&cc["id"])).unwrap(), Some(one));
        assert_eq!(db.holder_account(id(&lonely["id"])).unwrap(), None);
        let ordinary = frame(a, actor, &[a], "");
        let assignment = json!({"targets":{"selectors":[{"key":{"field":{"name":"account"}}}]},"assign":{"value":{"x":STANDARD.encode(both)}}});
        for operation in [
            call(
                "Holder",
                "Add",
                json!({"tenant":owner(a),"alias":"claimed","account":STANDARD.encode(both)}),
            ),
            call(
                "Holder",
                "Patch",
                json!({"ref":key(&cc),"dateUpdatedForce":true,"account":STANDARD.encode(both)}),
            ),
            call(
                "Holder",
                "Apply",
                json!({"ref":key(&cc),"patch":{"messageType":"app.Holder","delta":{"entries":[assignment]}}}),
            ),
        ] {
            let e = invoke(&mut db, &ordinary, operation).unwrap_err();
            assert_eq!(e.code, "invalid_argument");
            assert!(format!("{e:?}").contains("account"));
        }
        assert_eq!(
            db.memberships_for(one, None, "").unwrap()[0]["id"],
            cc["id"]
        );
        let acc = |action: &str| {
            serde_json::from_value::<Frame>(
                json!({"tenant":[],"actor":both,"account":both,"action":action}),
            )
            .unwrap()
        };
        let owned = "/app.HolderService/Memberships";
        let access = |db: &mut Store, f: Frame, op: Call, reads: Option<AccountReads>| {
            db.handle(Packet::Begin {
                id: "account".into(),
                frame: f,
            })?;
            let p = match reads {
                Some(reads) => Packet::AccountCall {
                    id: "account".into(),
                    call: op,
                    reads,
                },
                None => Packet::Call {
                    id: "account".into(),
                    call: op,
                },
            };
            let result = db.handle(p);
            db.handle(Packet::Finish {
                id: "account".into(),
                commit: result.is_ok(),
            })?;
            result
        };
        let mut inspected = 0;
        for (kind, p) in f.policies.as_object().unwrap() {
            for (rpc, method) in p["methods"].as_object().unwrap() {
                if method["policy"].get("account").is_some() {
                    continue;
                }
                for action in [String::new(), format!("/{kind}Service/{rpc}")] {
                    let e = access(
                        &mut db,
                        acc(&action),
                        Call {
                            entity: kind.clone(),
                            method: rpc.clone(),
                            request: json!({}),
                        },
                        None,
                    )
                    .unwrap_err();
                    assert_eq!(e.code, "permission_denied", "{kind}.{rpc}");
                    inspected += 1;
                }
            }
        }
        assert!(inspected > 100);
        let rows = access(
            &mut db,
            acc(owned),
            call("Holder", "Memberships", json!({})),
            None,
        )
        .unwrap();
        assert_eq!(rows["items"].as_array().unwrap().len(), 2);
        for (kind, p) in f.policies.as_object().unwrap() {
            if kind.ends_with("NotificationAccount")
                || kind.ends_with("Notification")
                || p["methods"].get("List").is_none()
            {
                continue;
            }
            let items = access(
                &mut db,
                acc(owned),
                Call {
                    entity: kind.clone(),
                    method: "List".into(),
                    request: json!({}),
                },
                None,
            );
            if let Ok(v) = items {
                assert_eq!(v["items"].as_array().unwrap().len(), 0, "{kind}");
            }
        }
        let query = || call("Holder", "Get", json!({"ref":key(&aa)}));
        assert_eq!(
            access(&mut db, acc(owned), query(), None).unwrap_err().code,
            "not_found"
        );
        let reads =
            AccountReads::default().allow("app.Holder", ReadFilter::AccountField("account".into()));
        let listed = access(
            &mut db,
            acc(owned),
            call("Holder", "List", json!({})),
            Some(reads.clone()),
        )
        .unwrap();
        assert_eq!(listed["items"].as_array().unwrap().len(), 2);
        assert_eq!(
            access(
                &mut db,
                acc(owned),
                call("Holder", "Get", json!({"ref":key(&cc)})),
                Some(reads)
            )
            .unwrap_err()
            .code,
            "not_found"
        );
        db.handle(Packet::Observe(Observation {
            frame: acc(owned),
            request: json!({}),
            input: "app.HolderMembershipsRequest".into(),
            output: "app.HolderListResponse".into(),
            answer: Some(listed),
            result: "ok".into(),
            reason: None,
            written: Default::default(),
        }))
        .unwrap();
        let records: Vec<_> = table_rows(&mut db, "app.Access")
            .into_iter()
            .filter(|r| r["action"] == owned)
            .collect();
        assert_eq!(records.len(), 2);
        for r in records {
            assert_eq!(r["actorId"], STANDARD.encode(both));
            assert_eq!(r["actorTenantId"], STANDARD.encode([0; 16]));
            assert_eq!(
                r["subjects"],
                json!([if r["tenantId"] == STANDARD.encode(a) {
                    &aa["id"]
                } else {
                    &bb["id"]
                }])
            );
        }
        db.call(&Scope::Internal, call("Holder", "Erase", key(&bb)))
            .unwrap();
        assert_eq!(db.memberships_for(both, None, "").unwrap().len(), 1);
        assert_eq!(db.holder_account(id(&bb["id"])).unwrap(), None);
    });
}

#[test]
fn transaction_composition_audit_hooks_and_null_free_documents() {
    backends(|f, url, _| {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let fail = Arc::new(AtomicBool::new(false));
        let gate = fail.clone();
        let mut db = f
            .open(url, &f.policies, false)
            .with_audit_filter(|kind, method| !(kind == "app.Cell" && method == "Add"))
            .unwrap()
            .with_write_observer(move |c| {
                if c.entity == "app.Joint" && gate.load(Ordering::SeqCst) {
                    Err(paydirt_store::Fault::internal("the queue is down"))
                } else {
                    Ok(())
                }
            })
            .unwrap();
        let (t, _) = world(&mut db);
        let actor = paydirt::mint(2);
        let client = frame(t, actor, &[t], "/app.RobotService/Move");
        let local = frame(t, actor, &[t], "");
        let silent = invoke(
            &mut db,
            &local,
            call("Cell", "Add", json!({"tenant":owner(t),"alias":"quiet"})),
        )
        .unwrap();
        assert!(
            table_rows(&mut db, "app.Audit")
                .iter()
                .all(|r| r["objectId"] != silent["id"])
        );
        assert_eq!(
            count_sql(url, r#"SELECT count(*) AS n FROM "pd_app__Cell""#),
            1
        );
        let create = |db: &mut Store, alias: &str| {
            let r = db.handle(Packet::Call {
                id: "compose".into(),
                call: call("Robot", "Add", json!({"tenant":owner(t),"alias":alias})),
            })?;
            db.handle(Packet::Call {
                id: "compose".into(),
                call: call("Joint", "Add", json!({"robot":key(&r),"alias":alias})),
            })?;
            Ok::<_, paydirt_store::Fault>(r)
        };
        let start = |db: &mut Store| {
            db.handle(Packet::Begin {
                id: "compose".into(),
                frame: client.clone(),
            })
            .unwrap()
        };
        start(&mut db);
        let first = create(&mut db, "arm-01").unwrap();
        db.handle(Packet::Finish {
            id: "compose".into(),
            commit: true,
        })
        .unwrap();
        let before = table_rows(&mut db, "app.Audit");
        let add = before
            .iter()
            .find(|r| r["objectId"] == first["id"])
            .unwrap();
        assert_eq!(add["actorId"], STANDARD.encode(actor));
        assert_eq!(add["tenantId"], STANDARD.encode(t));
        assert_eq!(add["action"], "/app.RobotService/Move");
        assert_eq!(add["value"], "");
        assert!(add["fields"].as_array().unwrap().contains(&json!(4)));
        let doc = message_json(f, "patch.Patch", &add["patch"]);
        let paths: Vec<_> = doc["delta"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                e["targets"]["selectors"][0]["key"]["field"]["name"]
                    .as_str()
                    .unwrap()
            })
            .collect();
        assert!(paths.contains(&"alias") && paths.contains(&"tenant") && !paths.contains(&"id"));
        start(&mut db);
        create(&mut db, "arm-02").unwrap();
        fail.store(true, Ordering::SeqCst);
        let e = create(&mut db, "arm-03").unwrap_err();
        assert_eq!(e.message, "the queue is down");
        assert!(
            db.handle(Packet::Finish {
                id: "compose".into(),
                commit: true
            })
            .is_err()
        );
        assert_eq!(table_rows(&mut db, "app.Robot").len(), 1);
        assert_eq!(
            count_sql(url, r#"SELECT count(*) AS n FROM "pd_app__Joint""#),
            1
        );
        assert_eq!(table_rows(&mut db, "app.Audit").len(), before.len());
        fail.store(false, Ordering::SeqCst);
        start(&mut db);
        create(&mut db, "arm-02").unwrap();
        create(&mut db, "arm-03").unwrap();
        db.handle(Packet::Finish {
            id: "compose".into(),
            commit: true,
        })
        .unwrap();
        assert_eq!(table_rows(&mut db, "app.Robot").len(), 3);
        assert_eq!(
            count_sql(url, r#"SELECT count(*) AS n FROM "pd_app__Joint""#),
            3
        );
        invoke(
            &mut db,
            &local,
            call(
                "Robot",
                "Patch",
                json!({"ref":key(&first),"alias":"next","dateUpdatedForce":true}),
            ),
        )
        .unwrap();
        let labels: Vec<_> = table_rows(&mut db, "app.Audit")
            .into_iter()
            .filter(|r| r["objectId"] == first["id"])
            .map(|r| r["action"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(labels.len(), 2);
        assert!(labels.iter().any(|v| v == "/app.RobotService/Patch"));
        assert!(labels.iter().any(|v| v == "/app.RobotService/Move"));
        let fleet = db
            .operator(call("Fleet", "Add", json!({"alias":"east"})))
            .unwrap();
        let reading = db
            .operator(call(
                "Reading",
                "Add",
                json!({"robot":key(&first),"celsius":21.5}),
            ))
            .unwrap();
        db.operator(call("Reading", "Erase", key(&reading)))
            .unwrap();
        let audits = table_rows(&mut db, "app.Audit");
        for object in [&fleet, &reading] {
            assert!(audits.iter().any(|r| r["objectId"] == object["id"]));
        }
        let model: paydirt_model::Model = serde_json::from_str(&f.model).unwrap();
        let e = model.entity("app.Audit").unwrap();
        let nulls = ["trace_id", "patch", "value"]
            .map(|n| format!("{} IS NULL", e.field(n).unwrap().column()))
            .join(" OR ");
        assert_eq!(
            count_sql(
                url,
                &format!(r#"SELECT count(*) AS n FROM "pd_app__Audit" WHERE {nulls}"#)
            ),
            0
        );
    });
}

#[test]
fn admission_checks_both_principals_and_redacts_requests_and_images_before_commit() {
    use paydirt_store::{
        Fault, PrincipalRow,
        admission::{Decision, Options},
    };
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    backends(|f, url, _| {
        let requests = Arc::new(Mutex::new(Vec::<Decision>::new()));
        let capture = requests.clone();
        let mut db = f
            .open(url, &f.policies, false)
            .with_admission(
                &f.schema,
                Options {
                    entities: vec!["app.Robot".into()],
                    timeout: Duration::from_millis(100),
                    ..Default::default()
                },
                move |d| {
                    let locked = d.previous.as_ref().is_some_and(|r| r["alias"] == "locked");
                    let slow = d.request["alias"] == "timeout";
                    capture.lock().unwrap().push(d);
                    if slow {
                        std::thread::sleep(Duration::from_millis(300));
                    }
                    if locked {
                        Err(Fault::denied("approved rows are immutable"))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap();
        let (a, b) = world(&mut db);
        let actor = paydirt::mint(2);
        let caller = paydirt::mint(2);
        let mut who = frame(a, actor, &[a], "");
        who.caller = Some(caller.to_vec());
        who.caller_tenant = Some(a.to_vec());
        who.presence = Some("operator-a".into());
        let principal = |alias: &str| {
            Some(PrincipalRow {
                entity: "app.Robot".into(),
                value: json!({"alias":alias,"secret":STANDARD.encode(alias)}),
            })
        };
        who.row = principal("person");
        who.caller_row = principal("device");
        let secret = STANDARD.encode(b"row-secret");
        let add = |name: &str| {
            call(
                "Robot",
                "Add",
                json!({"tenant":owner(a),"alias":name,"secret":secret}),
            )
        };
        let row = invoke(&mut db, &who, add("locked")).unwrap();
        let patch = call(
            "Robot",
            "Patch",
            json!({"ref":key(&row),"alias":"edited","secret":STANDARD.encode("new-secret"),"dateUpdatedForce":true}),
        );
        assert_eq!(
            invoke(&mut db, &who, patch).unwrap_err().code,
            "permission_denied"
        );
        {
            let list = requests.lock().unwrap();
            assert_eq!(list.len(), 2);
            for (i, d) in list.iter().enumerate() {
                assert_eq!(d.entity, "app.Robot");
                let principal = d.frame.as_ref().unwrap();
                assert_eq!(principal.actor.as_deref(), Some(actor.as_slice()));
                assert_eq!(principal.caller.as_deref(), Some(caller.as_slice()));
                assert_eq!(principal.presence.as_deref(), Some("operator-a"));
                for (r, alias) in [
                    (principal.row.as_ref(), "person"),
                    (principal.caller_row.as_ref(), "device"),
                ] {
                    let v = &r.unwrap().value;
                    assert_eq!(v["alias"], alias);
                    assert!(v.get("secret").is_none());
                }
                assert_eq!(d.previous.is_none(), i == 0);
                assert!(d.value.is_some());
                for v in [Some(&d.request), d.previous.as_ref(), d.value.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    assert!(v.get("secret").is_none());
                }
            }
        }
        assert_eq!(
            invoke(
                &mut db,
                &who,
                call("Robot", "Get", json!({"ref":key(&row)}))
            )
            .unwrap()["alias"],
            "locked"
        );
        assert_eq!(
            table_rows(&mut db, "app.Audit")
                .iter()
                .filter(|r| r["objectId"] == row["id"])
                .count(),
            1
        );
        let literal = if url.starts_with("postgres") {
            "decode('726f772d736563726574','hex')"
        } else {
            "X'726f772d736563726574'"
        };
        assert_eq!(
            count_sql(
                url,
                &format!(r#"SELECT count(*) AS n FROM "pd_app__Robot" WHERE f8={literal}"#)
            ),
            1
        );
        let stranger = frame(b, caller, &[b], "");
        assert_eq!(
            invoke(&mut db, &stranger, call("Robot", "Erase", key(&row))).unwrap()["erased"],
            false
        );
        let mut stamped = add("stamp");
        stamped.request["dateAttested"] = json!("2025-01-01T00:00:00Z");
        assert_eq!(
            invoke(&mut db, &who, stamped).unwrap_err().code,
            "invalid_argument"
        );
        assert_eq!(requests.lock().unwrap().len(), 2);
        assert_eq!(
            invoke(&mut db, &who, add("timeout")).unwrap_err().code,
            "deadline_exceeded"
        );
        let mut raw = add("bypass").request;
        raw["id"] = json!(
            STANDARD.encode(paydirt::mint(
                f.policies["app.Robot"]["entity"]["domain"]
                    .as_u64()
                    .unwrap() as u8
            ))
        );
        assert_eq!(
            db.seed(vec![("app.Robot".into(), raw)]).unwrap_err().code,
            "failed_precondition"
        );
        assert_eq!(table_rows(&mut db, "app.Robot").len(), 1);
    });
}

#[test]
fn edge_reads_and_stamps_apply_to_overlay_fields_moved_edges_and_cached_comparisons() {
    backends(|f, url, _| {
        let mut db = f.open(url, &f.policies, false);
        let (a, b) = world(&mut db);
        let actor = paydirt::mint(2);
        let mine = frame(a, actor, &[a], "");
        let add = |db: &mut Store, kind: &str, t: [u8; 16], alias: &str| {
            db.operator(call(kind, "Add", json!({"tenant":owner(t),"alias":alias})))
                .unwrap()
        };
        let bot = add(&mut db, "Robot", a, "mine");
        let other = add(&mut db, "Robot", b, "theirs");
        let foreign = table_rows(&mut db, "app.Audit")
            .into_iter()
            .filter(|r| r["objectId"] == other["id"])
            .collect::<Vec<_>>();
        assert_eq!(foreign.len(), 1);
        assert_eq!(foreign[0]["tenantId"], STANDARD.encode(b));
        assert_ne!(foreign[0]["actorTenantId"], foreign[0]["tenantId"]);
        let patch = message_json(f, "patch.Patch", &foreign[0]["patch"]);
        let names: Vec<_> = patch["delta"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| &e["targets"]["selectors"][0]["key"]["field"]["name"])
            .collect();
        assert!(names.contains(&&json!("alias")) && !names.contains(&&json!("id")));
        let mut joints = Vec::new();
        for r in [&bot, &other] {
            joints.push(
                db.operator(call(
                    "Joint",
                    "Add",
                    json!({"robot":key(r),"alias":"elbow"}),
                ))
                .unwrap(),
            );
        }
        for (kind, local, away) in [("Robot", &bot, &other), ("Joint", &joints[0], &joints[1])] {
            let get = |row: &Value| call(kind, "Get", json!({"ref":key(row)}));
            assert_eq!(
                invoke(&mut db, &mine, get(local)).unwrap()["id"],
                local["id"]
            );
            assert_eq!(
                invoke(&mut db, &mine, get(away)).unwrap_err().code,
                "not_found"
            );
        }
        let listed = invoke(&mut db, &mine, call("Robot", "List", json!({}))).unwrap();
        assert_eq!(listed["items"].as_array().unwrap().len(), 1);
        let near = add(&mut db, "Cell", a, "near");
        let far = add(&mut db, "Cell", b, "far");
        let set_cell = |r: &Value| {
            call(
                "Robot",
                "Patch",
                json!({"ref":key(&bot),"cell":key(r),"dateUpdatedForce":true}),
            )
        };
        assert_eq!(
            invoke(&mut db, &mine, set_cell(&far)).unwrap_err().code,
            "not_found"
        );
        let mut moving = mine.clone();
        moving.action = "/app.RobotService/Move".into();
        let moved = invoke(&mut db, &moving, set_cell(&near)).unwrap();
        assert_eq!(moved["cell"]["id"], near["id"]);
        let trail = table_rows(&mut db, "app.Audit");
        assert!(
            trail
                .iter()
                .any(|r| r["objectId"] == bot["id"] && r["action"] == moving.action)
        );
        let holder = |r: &Value| {
            call(
                "Holder",
                "Add",
                json!({"tenant":owner(a),"alias":"minder","robot":key(r)}),
            )
        };
        assert_eq!(
            invoke(&mut db, &mine, holder(&other)).unwrap_err().code,
            "not_found"
        );
        let h = invoke(&mut db, &mine, holder(&bot)).unwrap();
        assert_eq!(h["robot"]["id"], bot["id"]);
        let change = |r: &Value| {
            call(
                "Holder",
                "Patch",
                json!({"ref":key(&h),"robot":key(r),"dateUpdatedForce":true}),
            )
        };
        assert_eq!(
            invoke(&mut db, &mine, change(&other)).unwrap_err().code,
            "not_found"
        );
        assert!(invoke(&mut db, &mine, change(&bot)).is_ok());
        for (kind, field, value) in [
            ("Holder", "directory", json!("elsewhere")),
            ("Robot", "dateAttested", json!("2025-01-02T03:04:05Z")),
        ] {
            let stamped = || {
                call(
                    kind,
                    "Add",
                    json!({"tenant":owner(a),"alias":"imported",field:value}),
                )
            };
            let err = invoke(&mut db, &mine, stamped()).unwrap_err();
            assert_eq!(err.code, "invalid_argument");
            assert!(
                err.field.as_deref()
                    == Some(if kind == "Robot" {
                        "date_attested"
                    } else {
                        "directory"
                    }),
                "{err:?}"
            );
            let stamped = db.operator(stamped()).unwrap();
            assert!(stamped[field].is_string());
            let plain = invoke(
                &mut db,
                &mine,
                call(kind, "Add", json!({"tenant":owner(a),"alias":"plain"})),
            )
            .unwrap();
            assert!(
                invoke(
                    &mut db,
                    &mine,
                    call(
                        kind,
                        "Patch",
                        json!({"ref":key(&plain),field:value,"dateUpdatedForce":true})
                    )
                )
                .unwrap()[field]
                    .is_string()
            );
        }
        let region = || call("Tenant", "Add", json!({"alias":"korea","region":"kr"}));
        assert_eq!(
            invoke(&mut db, &mine, region()).unwrap_err().code,
            "unimplemented"
        );
        assert_eq!(db.operator(region()).unwrap()["region"], "kr");
        let next = add(&mut db, "Robot", a, "also-mine");
        let apply = |to: &Value| {
            call(
                "Holder",
                "Apply",
                json!({"ref":key(&h),"patch":{"messageType":"app.Holder","delta":{"entries":[{"path":{"segments":[{"field":{"name":"robot"}}]},"targets":{"selectors":[{"key":{"field":{"name":"id"}}}]},"assign":{"value":{"x":to["id"]}}}]}}}),
            )
        };
        assert_eq!(
            invoke(&mut db, &mine, apply(&other)).unwrap_err().code,
            "not_found"
        );
        assert_eq!(
            db.operator(call("Holder", "Get", json!({"ref":key(&h)})))
                .unwrap()["robot"]["id"],
            bot["id"]
        );
        assert_eq!(
            invoke(&mut db, &mine, apply(&next)).unwrap()["robot"]["id"],
            next["id"]
        );
        let reading = invoke(
            &mut db,
            &mine,
            call("Reading", "Add", json!({"robot":key(&bot),"celsius":21.5})),
        )
        .unwrap();
        assert_eq!(reading["tenantId"], STANDARD.encode(a));
        let follower = add(&mut db, "Robot", a, "follower");
        let both = frame(a, actor, &[a, b], "/app.PairingService/Add");
        db.handle(Packet::Begin {
            id: "pair".into(),
            frame: both,
        })
        .unwrap();
        let pair = |follow: &Value| Packet::Call {
            id: "pair".into(),
            call: call(
                "Pairing",
                "Add",
                json!({"lead":key(&bot),"follow":key(follow)}),
            ),
        };
        db.handle(pair(&follower)).unwrap();
        let err = db.handle(pair(&other)).unwrap_err();
        assert_eq!(err.code, "invalid_argument");
        assert!(err.message.contains("tenant"));
        db.handle(Packet::Finish {
            id: "pair".into(),
            commit: false,
        })
        .unwrap();
    });
}

#[cfg(feature = "postgres")]
#[test]
fn concurrent_entitlement_limit_serializes_a_write_while_an_unlocked_read_allows_both() {
    use std::{
        sync::{Barrier, mpsc},
        thread,
        time::Duration,
    };
    backends(|f, url, _| {
        if !url.starts_with("postgres") {
            return;
        }
        let mut setup = f.open(url, &f.policies, false);
        let tenants = world(&mut setup);
        for (tenant, lock) in [(tenants.0, true), (tenants.1, false)] {
            let key = "apptest.robots";
            let grant = json!({"tenantId":STANDARD.encode(tenant),"key":key,"id":STANDARD.encode(tenant),"version":1,"limit":"1","dateExpires":"2099-03-04T00:00:00Z"});
            assert_eq!(
                setup.apply_grants(None, &[grant], false).unwrap().applied,
                1
            );
            let ready = Barrier::new(3);
            let (acquired, first_acquired) = mpsc::channel();
            let (finished, second_done) = mpsc::channel();
            let (release, wait) = mpsc::channel();
            let counted = Barrier::new(2);
            let do_write =
                |db: &mut Store,
                 name: &str,
                 signal: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>| {
                    db.transaction(&Scope::Tenant(tenant.to_vec()), |tx| {
                        let allow = tx.entitlement(key, lock, chrono::Utc::now())?;
                        let rows = tx.call(call("Robot", "List", json!({})))?;
                        let count = rows["items"].as_array().map_or(0, Vec::len) as i64;
                        if let Some((ready, wait)) = signal {
                            ready.send(()).unwrap();
                            wait.recv_timeout(Duration::from_secs(8))
                                .expect("release the first transaction");
                        }
                        if !lock {
                            counted.wait();
                        }
                        if !allow.allows(count + 1) {
                            return Err(paydirt_store::Fault::denied("limit reached"));
                        }
                        tx.call(call(
                            "Robot",
                            "Add",
                            json!({"tenant":owner(tenant),"alias":name}),
                        ))?;
                        Ok(())
                    })
                };
            thread::scope(|s| {
                let (start, second_start) = mpsc::channel();
                let r = &ready;
                let write = &do_write;
                let first = s.spawn(move || {
                    let mut db = f.open(url, &f.policies, false);
                    r.wait();
                    write(&mut db, "first", lock.then_some((acquired, wait)))
                });
                let other = s.spawn(move || {
                    let mut db = f.open(url, &f.policies, false);
                    r.wait();
                    second_start.recv_timeout(Duration::from_secs(8)).unwrap();
                    let result = write(&mut db, "second", None);
                    finished.send(result.clone()).unwrap();
                    result
                });
                ready.wait();
                if lock {
                    first_acquired.recv_timeout(Duration::from_secs(8)).unwrap();
                }
                start.send(()).unwrap();
                if lock {
                    assert!(matches!(
                        second_done.recv_timeout(Duration::from_millis(300)),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ));
                    release.send(()).unwrap();
                }
                first.join().unwrap().unwrap();
                let res = other.join().unwrap();
                if lock {
                    assert_eq!(res.unwrap_err().code, "permission_denied");
                } else {
                    res.unwrap();
                }
            });
            let rows = setup
                .call(
                    &Scope::Tenant(tenant.to_vec()),
                    call("Robot", "List", json!({})),
                )
                .unwrap();
            assert_eq!(
                rows["items"].as_array().unwrap().len(),
                if lock { 1 } else { 2 }
            );
        }
    });
}
