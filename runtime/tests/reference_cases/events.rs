use super::walls::{call, frame, key, owner};
use super::*;
use futures_util::future::BoxFuture;
use paydirt::watch::{Broker, Event, Interest, Subscription};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Publisher {
    attempts: Mutex<Vec<Event>>,
    refuse: bool,
}
impl Broker for Publisher {
    fn subscribe(&self, _: Interest) -> BoxFuture<'_, Result<Subscription, String>> {
        Box::pin(async { Err("publish-only test broker".into()) })
    }
    fn publish(&self, event: Event) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.attempts.lock().unwrap().push(event);
            if self.refuse {
                Err("offline".into())
            } else {
                Ok(())
            }
        })
    }
}
fn commit(store: &mut Store, f: &Frame, operations: Vec<Call>) -> (Vec<Value>, Event) {
    store
        .handle(Packet::Begin {
            id: "event".into(),
            frame: f.clone(),
        })
        .unwrap();
    let results = operations
        .into_iter()
        .map(|call| {
            store
                .handle(Packet::Call {
                    id: "event".into(),
                    call,
                })
                .unwrap()
        })
        .collect();
    let event = store
        .handle(Packet::Finish {
            id: "event".into(),
            commit: true,
        })
        .unwrap();
    (results, serde_json::from_value(event).unwrap())
}
fn recorder(f: &Fixture, url: &str, previous: bool) -> Store {
    f.open(url, &f.policies, false)
        .with_outbox(true)
        .with_resources(
            paydirt_store::resource::Options {
                entities: ["app.Robot".into()].into(),
                previous,
            },
            &f.descriptors,
            &f.schema,
        )
        .unwrap()
}

#[test]
fn resource_changes_keep_order_identity_and_replay_the_same_redacted_committed_bytes() {
    backends(|f, url, raw| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut setup = f.open(url, &f.policies, false);
        let t = provision(&mut setup);
        drop(setup);
        for previous in [false, true] {
            let mut db = recorder(f, url, previous);
            let actor = paydirt::mint(2);
            let add = |alias: &str| json!({"tenant":owner(t),"alias":alias,"secret":STANDARD.encode("never publish this verifier")});
            let outer = frame(t, actor, &[t], "/app.RobotService/AddMany");
            let (rows, added) = commit(
                &mut db,
                &outer,
                vec![call(
                    "Robot",
                    "AddMany",
                    json!({"items":[add(if previous{"aa"}else{"bb"}),add(if previous{"cc"}else{"dd"})]}),
                )],
            );
            assert_eq!(added.changes.len(), 2);
            assert_ne!(added.changes[0].resource, added.changes[1].resource);
            let row = &rows[0]["items"][0];
            let delta = frame(t, actor, &[t], "/app.RobotService/Patch");
            let (_, changed) = commit(
                &mut db,
                &delta,
                vec![call(
                    "Robot",
                    "Patch",
                    json!({"ref":key(row),"alias":if previous{"after-yes"}else{"after-no"},"dateUpdatedForce":true,"secret":STANDARD.encode("a second verifier")}),
                )],
            );
            let reader =
                paydirt_store::resource::PublicEvents::new(&f.descriptors, &f.schema).unwrap();
            let mut recorded = Vec::new();
            for event in [&added, &changed] {
                for c in &event.changes {
                    let bytes = STANDARD.decode(&c.resource).unwrap();
                    let plain = String::from_utf8_lossy(&bytes);
                    assert!(!plain.contains("verifier"));
                    let (msg, public) = reader.public(&bytes).unwrap();
                    assert_eq!(bytes, public);
                    assert_eq!(msg["method"], event.method);
                    assert_eq!(msg["callMethod"], event.method);
                    if event.method.ends_with("Patch") {
                        assert_eq!(msg["previous"].is_object(), previous);
                        assert_eq!(
                            msg["value"]["alias"],
                            if previous { "after-yes" } else { "after-no" }
                        );
                    }
                    recorded.push(c.resource.clone());
                }
            }
            let pending = db.queued_events(80).unwrap();
            let queued: Vec<_> = pending
                .iter()
                .flat_map(|r| r.event.changes.iter().map(|c| c.resource.clone()))
                .collect();
            assert_eq!(queued, recorded);
            let fail = Publisher {
                refuse: true,
                ..Default::default()
            };
            assert!(rt.block_on(db.drain_once(&fail, 3)).is_err());
            let retry = Publisher::default();
            assert_eq!(rt.block_on(db.drain_once(&retry, 3)).unwrap(), 3);
            assert_eq!(
                fail.attempts.lock().unwrap()[0],
                retry.attempts.lock().unwrap()[0]
            );
            assert!(db.queued_events(10).unwrap().is_empty());
        }
        let mut db = recorder(f, url, true);
        let before = table_rows(&mut db, "app.Robot").len();
        let table = r#""pd_app__Outbox""#;
        if url.starts_with("postgres") {
            raw(&format!(
                "ALTER TABLE {table} ADD CONSTRAINT reject_outbox CHECK (false)"
            ));
        } else {
            raw(&format!(
                "CREATE TRIGGER reject_outbox BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT,'outbox unavailable'); END"
            ));
        }
        let who = frame(t, paydirt::mint(2), &[t], "/app.RobotService/Add");
        assert!(
            super::walls::invoke(
                &mut db,
                &who,
                call("Robot", "Add", json!({"tenant":owner(t),"alias":"blocked"}))
            )
            .is_err()
        );
        assert_eq!(table_rows(&mut db, "app.Robot").len(), before);
    });
}

#[test]
fn publication_policy_and_historical_scope_use_the_correct_images_on_both_paths() {
    backends(|f, url, raw| {
        let calls = Arc::new(Mutex::new(vec![]));
        let seen = calls.clone();
        let mut db = recorder(f, url, true)
            .with_resource_policy(move |p| {
                assert_eq!(p.entity, "app.Robot");
                assert_eq!(p.outer, "/jobs/Import");
                let alias = p.value.unwrap()["alias"].as_str().unwrap();
                seen.lock().unwrap().push(alias.to_owned());
                Ok(alias == "published")
            })
            .unwrap();
        let t = provision(&mut db);
        let actor = paydirt::mint(2);
        let outer = frame(t, actor, &[t], "/jobs/Import");
        let add = |alias: &str| {
            call(
                "Robot",
                "Add",
                json!({"tenant":owner(t),"alias":alias,"secret":STANDARD.encode("secret")}),
            )
        };
        let (rows, event) = commit(&mut db, &outer, vec![add("suppressed"), add("published")]);
        assert_eq!(*calls.lock().unwrap(), ["suppressed", "published"]);
        let live: Vec<_> = event
            .changes
            .iter()
            .filter(|c| !c.resource.is_empty())
            .collect();
        assert_eq!(live.len(), 1);
        let queued = db.queued_events(10).unwrap();
        assert_eq!(
            queued
                .iter()
                .flat_map(|r| r.event.changes.iter())
                .filter(|c| !c.resource.is_empty())
                .map(|c| &c.resource)
                .collect::<Vec<_>>(),
            vec![&live[0].resource]
        );
        assert_eq!(table_rows(&mut db, "app.Audit").len(), 2);
        let mut db = f
            .open(url, &f.policies, false)
            .with_scopes(vec![Arc::new(|e, _| {
                Ok(if e == "app.Robot" {
                    vec![("alias".into(), json!("published"))]
                } else {
                    vec![]
                })
            })])
            .unwrap();
        let project = |db: &mut Store, scope: &Scope, kind: &str, row: &Value| {
            db.call(scope, call(kind, "$resource_visible", row.clone()))
        };
        let scoped = Scope::Tenant(t.to_vec());
        assert_eq!(project(&mut db, &scoped, "Robot", &rows[1]).unwrap(), true);
        let changed = db
            .operator(call(
                "Robot",
                "Patch",
                json!({"ref":key(&rows[1]),"alias":"denied","dateUpdatedForce":true}),
            ))
            .unwrap();
        assert_eq!(project(&mut db, &scoped, "Robot", &changed).unwrap(), false);
        db.operator(call("Robot", "Erase", key(&rows[1]))).unwrap();
        assert_eq!(project(&mut db, &scoped, "Robot", &rows[1]).unwrap(), true);
        let mut db = f
            .open(url, &f.policies, false)
            .with_scopes(vec![Arc::new(|e, _| {
                Ok(if e == "app.Robot" {
                    vec![("secret".into(), Value::Null)]
                } else {
                    vec![]
                })
            })])
            .unwrap();
        assert!(project(&mut db, &scoped, "Robot", &rows[0]).is_err());
        let mut db = f.open(url, &f.policies, false);
        let embedded = db
            .operator(call(
                "Pairing",
                "Add",
                json!({"lead":key(&rows[0]),"follow":key(&rows[0])}),
            ))
            .unwrap();
        db.operator(call("Pairing", "Erase", key(&embedded)))
            .unwrap();
        assert_eq!(
            project(&mut db, &scoped, "Pairing", &embedded).unwrap(),
            true
        );
        let model: paydirt_model::Model = serde_json::from_str(&f.model).unwrap();
        let e = model.entity("app.Robot").unwrap();
        let k = e.key().unwrap().column();
        let hex = hex::encode(id(&rows[0]["id"]));
        let value = if url.starts_with("postgres") {
            format!("decode('{hex}','hex')")
        } else {
            format!("X'{hex}'")
        };
        raw(&format!(r#"DELETE FROM "pd_app__Robot" WHERE {k}={value}"#));
        assert_eq!(
            project(&mut db, &scoped, "Pairing", &embedded).unwrap(),
            false
        );
        assert_eq!(
            project(&mut db, &Scope::Internal, "Pairing", &embedded).unwrap(),
            true
        );
    });
}

fn mark_stale(f: &Fixture, url: &str, raw: &dyn Fn(&str), notice: &Value) {
    let m: paydirt_model::Model = serde_json::from_str(&f.model).unwrap();
    let e = m.entity("app.NotificationRecord").unwrap();
    let date = e.field("date_created").unwrap().column();
    let key = e.key().unwrap().column();
    let hex = hex::encode(id(&notice["id"]));
    let id = if url.starts_with("postgres") {
        format!("decode('{hex}','hex')")
    } else {
        format!("X'{hex}'")
    };
    raw(&format!(
        r#"UPDATE "pd_app__NotificationRecord" SET {date}='2000-01-01T00:00:00Z' WHERE {key}={id}"#
    ));
}
fn notice_frame(t: [u8; 16], reader: [u8; 16]) -> Frame {
    let mut f = frame(t, reader, &[t], "");
    f.topics = paydirt_store::TopicGrants {
        read: vec!["n:jobs".into()],
        publish: vec!["*".into()],
        producer: Some("producer".into()),
    };
    f
}
#[test]
fn account_notifications_keep_global_ownership_independent_receipts_and_expire_atomically() {
    backends(|f, url, raw| {
        let mut db = f.open(url, &f.policies, false).with_outbox(true);
        let account = paydirt::mint(2);
        let other = paydirt::mint(2);
        let everybody = json!({"name":"everyone"});
        let personal = json!({"resourceId":STANDARD.encode(account)});
        let publish = |db: &mut Store, topic: &Value, k: &str| {
            db.call(
                &Scope::Internal,
                call(
                    "NotificationAccount",
                    "Add",
                    json!({"topic":topic,"kind":"invitation.received","idempotencyKey":k}),
                ),
            )
            .unwrap()
        };
        let invited = publish(&mut db, &personal, "invited");
        let shared = publish(&mut db, &everybody, "notice");
        assert!(invited["tenantId"].is_null());
        assert_eq!(publish(&mut db, &personal, "invited"), invited);
        let records = stored_count(f, "NotificationRecord", url);
        assert_eq!(records, 2);
        let acc = |id: [u8; 16]| {
            serde_json::from_value(json!({"tenant":[],"actor":id,"account":id,"action":"","topics":{"read":["self","n:everyone"]}})).unwrap()
        };
        let op = |db: &mut Store, reader: [u8; 16], method: &str, req: Value| {
            super::walls::invoke(db, &acc(reader), call("NotificationAccount", method, req))
        };
        let topics = json!({"topics":[everybody,personal]});
        let page = op(&mut db, account, "Read", topics.clone()).unwrap();
        assert_eq!(page["readerId"], STANDARD.encode(account));
        assert_eq!(page["items"].as_array().unwrap().len(), 2);
        let count = op(
            &mut db,
            account,
            "Count",
            json!({"topics":topics["topics"],"filter":{"unreadOnly":true}}),
        )
        .unwrap();
        assert_eq!(count["count"].as_str().unwrap_or(""), "2");
        assert_eq!(
            op(&mut db, account, "Get", json!({"ref":key(&invited)})).unwrap(),
            invited
        );
        assert_eq!(
            op(&mut db, other, "Read", topics).unwrap_err().code,
            "permission_denied"
        );
        assert_eq!(
            op(&mut db, other, "Get", json!({"ref":key(&invited)}))
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            op(&mut db, other, "Read", json!({"topics":[everybody]})).unwrap()["items"][0]["notification"],
            shared
        );
        assert_eq!(
            op(
                &mut db,
                account,
                "Mark",
                json!({"refs":[key(&invited),key(&shared)]})
            )
            .unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let retry = op(&mut db, account, "Mark", json!({"refs":[key(&invited)]})).unwrap();
        assert_eq!(retry["items"][0]["readerId"], STANDARD.encode(account));
        assert!(retry["items"][0]["tenantId"].is_null());
        let own = op(&mut db, other, "Mark", json!({"refs":[key(&shared)]})).unwrap();
        assert_ne!(retry["items"][0]["id"], own["items"][0]["id"]);
        let lines = table_rows(&mut db, "app.Audit");
        assert_eq!(
            lines
                .iter()
                .filter(
                    |v| v["objectId"] == invited["id"] && v["tenantId"] == STANDARD.encode([0; 16])
                )
                .count(),
            1
        );
        assert!(
            lines
                .iter()
                .all(|v| v["objectId"] != retry["items"][0]["id"]
                    && v["objectId"] != own["items"][0]["id"])
        );
        mark_stale(f, url, raw, &invited);
        assert_eq!(db.expire_notifications(Some([0; 16]), 100).unwrap(), 1);
        assert_eq!(stored_count(f, "NotificationRecord", url), 1);
        assert_eq!(stored_count(f, "NotificationReceiptRecord", url), 2);
        assert_eq!(
            op(&mut db, account, "Get", json!({"ref":key(&invited)}))
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            op(&mut db, account, "Get", json!({"ref":key(&shared)})).unwrap(),
            shared
        );
        assert!(
            table_rows(&mut db, "app.Audit")
                .iter()
                .any(|r| r["objectId"] == invited["id"]
                    && r["action"] == "payday.notify.expire"
                    && r["tenantId"] == STANDARD.encode([0; 16]))
        );
    });
}
fn stored_count(f: &Fixture, name: &str, url: &str) -> i64 {
    let model: paydirt_model::Model = serde_json::from_str(&f.model).unwrap();
    let e = model.entity(&format!("app.{name}")).unwrap();
    count_sql(
        url,
        &format!(
            "SELECT count(*) AS n FROM {}",
            paydirt_model::quote(&e.table())
        ),
    )
}

#[test]
fn notification_mark_retries_in_reverse_order_and_expiry_waits_for_a_pending_receipt() {
    use std::{
        sync::{Barrier, mpsc},
        thread,
        time::Duration,
    };
    backends(|f, url, raw| {
        let mut db = f.open(url, &f.policies, false);
        let t = provision(&mut db);
        let actor = paydirt::mint(2);
        let who = notice_frame(t, actor);
        let invoke =
            |db: &mut Store, m: &str, q| super::walls::invoke(db, &who, call("Notification", m, q));
        let add = |db: &mut Store, k: &str, topic: &str| {
            invoke(db,"Add",json!({"tenantId":STANDARD.encode(t),"topic":{"name":topic},"kind":"work.done","idempotencyKey":k})).unwrap()
        };
        let n = add(&mut db, "one", "jobs");
        let d = add(&mut db, "two", "jobs");
        let hidden = add(&mut db, "hidden", "other");
        let mark = |ids: Vec<&Value>| json!({"tenantId":STANDARD.encode(t),"refs":ids.iter().map(|n|key(n)).collect::<Vec<_>>()});
        assert_eq!(
            invoke(&mut db, "Mark", mark(vec![&n, &hidden]))
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(stored_count(f, "NotificationReceiptRecord", url), 0);
        let ready = Barrier::new(2);
        thread::scope(|s| {
            let first = s.spawn(|| {
                let mut db = f.open(url, &f.policies, false);
                ready.wait();
                invoke(&mut db, "Mark", mark(vec![&n, &d])).unwrap()
            });
            let second = s.spawn(|| {
                let mut db = f.open(url, &f.policies, false);
                ready.wait();
                invoke(&mut db, "Mark", mark(vec![&d, &n])).unwrap()
            });
            let a = first.join().unwrap();
            let b = second.join().unwrap();
            assert_eq!(a["items"][0], b["items"][1]);
            assert_eq!(a["items"][1], b["items"][0]);
        });
        assert_eq!(stored_count(f, "NotificationReceiptRecord", url), 2);
        let stale = add(&mut db, "stale", "jobs");
        mark_stale(f, url, raw, &stale);
        let ready = Barrier::new(2);
        let (go, start) = mpsc::channel();
        let (done, completed) = mpsc::channel();
        thread::scope(|s| {
            let r = &ready;
            let exp = s.spawn(move || {
                let mut db = f.open(url, &f.policies, false);
                r.wait();
                start.recv_timeout(Duration::from_secs(5)).unwrap();
                let n = db.expire_notifications(Some(t), 20).unwrap();
                done.send(n).unwrap();
            });
            ready.wait();
            db.handle(Packet::Begin {
                id: "mark".into(),
                frame: who.clone(),
            })
            .unwrap();
            db.handle(Packet::Call {
                id: "mark".into(),
                call: call("Notification", "Mark", mark(vec![&stale])),
            })
            .unwrap();
            go.send(()).unwrap();
            assert!(matches!(
                completed.recv_timeout(Duration::from_millis(220)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            db.handle(Packet::Finish {
                id: "mark".into(),
                commit: true,
            })
            .unwrap();
            assert_eq!(completed.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
            exp.join().unwrap();
        });
        assert_eq!(stored_count(f, "NotificationReceiptRecord", url), 2);
        assert_eq!(
            invoke(&mut db, "Get", json!({"ref":key(&stale)}))
                .unwrap_err()
                .code,
            "not_found"
        );
    });
}

#[test]
fn notification_routing_survives_removal_and_publications_share_the_business_transaction() {
    use std::{sync::Barrier, thread};
    backends(|f, url, raw| {
        let mut db = f.open(url, &f.policies, false).with_outbox(true);
        let tenant = provision(&mut db);
        let actor = paydirt::mint(2);
        let mut who = notice_frame(tenant, actor);
        who.action = "/jobs/Complete".into();
        let complete = |db: &mut Store, commit: bool| {
            db.handle(Packet::Begin {
                id: "work".into(),
                frame: who.clone(),
            })
            .unwrap();
            let run = |db: &mut Store, op| {
                db.handle(Packet::Call {
                    id: "work".into(),
                    call: op,
                })
                .unwrap()
            };
            let source = run(
                db,
                call(
                    "Robot",
                    "Add",
                    json!({"tenant":owner(tenant),"alias":"machine"}),
                ),
            );
            let result = run(
                db,
                call(
                    "Notification",
                    "Add",
                    json!({"tenantId":STANDARD.encode(tenant),"topic":{"resourceId":source["id"]},"sourceId":source["id"],"kind":"job.done","idempotencyKey":"complete"}),
                ),
            );
            let event = db
                .handle(Packet::Finish {
                    id: "work".into(),
                    commit,
                })
                .unwrap();
            (source, result, event)
        };
        complete(&mut db, false);
        assert!(table_rows(&mut db, "app.Robot").is_empty());
        assert_eq!(stored_count(f, "NotificationRecord", url), 0);
        assert!(db.queued_events(60).unwrap().is_empty());
        let (source, notice, event) = complete(&mut db, true);
        assert_eq!(stored_count(f, "NotificationRecord", url), 1);
        assert_eq!(stored_count(f, "NotificationReceiptRecord", url), 0);
        let public: Event = serde_json::from_value(event).unwrap();
        let original = public
            .changes
            .iter()
            .find(|c| !c.signal.is_empty())
            .unwrap();
        let signal: Value =
            serde_json::from_slice(&STANDARD.decode(&original.signal).unwrap()).unwrap();
        assert_eq!(signal["Tenant"], paydirt::format_id(&tenant).unwrap());
        assert_eq!(
            signal["Topics"][0]["Resource"],
            paydirt::format_id(&id(&source["id"])).unwrap()
        );
        assert!(
            original
                .routes
                .as_ref()
                .is_some_and(|routes| routes.len() == 1)
        );
        let m: paydirt_model::Model = serde_json::from_str(&f.model).unwrap();
        let e = m.entity("app.Robot").unwrap();
        let x = hex::encode(id(&source["id"]));
        let lit = if url.starts_with("postgres") {
            format!("decode('{x}','hex')")
        } else {
            format!("X'{x}'")
        };
        raw(&format!(
            "DELETE FROM {} WHERE {}={lit}",
            paydirt_model::quote(&e.table()),
            e.key().unwrap().column()
        ));
        let queue = db.queued_events(60).unwrap();
        assert!(
            queue
                .iter()
                .flat_map(|e| &e.event.changes)
                .any(|c| c.signal == original.signal && c.routes == original.routes)
        );
        assert!(
            table_rows(&mut db, "app.Audit")
                .iter()
                .any(|r| r["objectId"] == notice["id"])
        );
        let both = Barrier::new(2);
        let add = || {
            let mut db = f.open(url, &f.policies, false);
            both.wait();
            super::walls::invoke(&mut db,&who,call("Notification","Add",json!({"tenantId":STANDARD.encode(tenant),"topic":{"name":"jobs"},"kind":"job.done","idempotencyKey":"race"}))).unwrap()
        };
        thread::scope(|s| {
            let a = s.spawn(add);
            let b = s.spawn(add);
            assert_eq!(a.join().unwrap(), b.join().unwrap());
        });
        assert_eq!(stored_count(f, "NotificationRecord", url), 2);
        assert_eq!(stored_count(f, "NotificationReceiptRecord", url), 0);
        let pending = db.queued_events(100).unwrap().len();
        let mut immediate = f.open(url, &f.policies, false).with_outbox(false);
        super::walls::invoke(
            &mut immediate,
            &who,
            call(
                "Robot",
                "Add",
                json!({"tenant":owner(tenant),"alias":"direct"}),
            ),
        )
        .unwrap();
        assert_eq!(immediate.queued_events(100).unwrap().len(), pending);
    });
}
