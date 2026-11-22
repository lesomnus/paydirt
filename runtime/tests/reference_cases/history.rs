use super::walls::{call, frame, owner, world};
use super::*;
use paydirt_store::{Fault, Inputs, past_expiry::Window};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
fn moment(n: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_700_000_000 + n, 0).unwrap()
}
fn timed(db: Store) -> (Store, Arc<AtomicI64>) {
    let c = Arc::new(AtomicI64::new(0));
    let t = c.clone();
    (
        db.with_inputs(Inputs::new(
            move || moment(t.load(Ordering::SeqCst)),
            paydirt::mint,
        )),
        c,
    )
}
fn transaction<T>(
    db: &mut Store,
    who: &Frame,
    f: impl FnOnce(&mut Store) -> paydirt_store::Result<T>,
) -> paydirt_store::Result<T> {
    db.handle(Packet::Begin {
        id: "history".into(),
        frame: who.clone(),
    })?;
    let answer = f(db);
    db.handle(Packet::Finish {
        id: "history".into(),
        commit: answer.is_ok(),
    })?;
    answer
}
fn op(db: &mut Store, entity: &str, method: &str, request: Value) -> paydirt_store::Result<Value> {
    db.handle(Packet::Call {
        id: "history".into(),
        call: call(entity, method, request),
    })
}
fn begin(
    db: &mut Store,
    entity: &str,
    subject: &Value,
    kind: &str,
    ty: &str,
) -> paydirt_store::Result<Value> {
    op(
        db,
        entity,
        "$begin_event",
        json!({"subject":subject,"kind":kind,"reason":"due","dateStart":moment(1),"value":{"@type":format!("type.googleapis.com/app.{ty}")}}),
    )
}
fn list(db: &mut Store, tenant: [u8; 16], entity: &str) -> Vec<Value> {
    db.call(
        &Scope::Tenant(tenant.to_vec()),
        call(entity, "List", json!({"pageSize":1000})),
    )
    .unwrap()["items"]
        .as_array()
        .unwrap()
        .clone()
}
fn statements(db: &mut Store, t: [u8; 16], req: Value) -> Vec<Value> {
    db.call(&Scope::Tenant(t.to_vec()), call("Fact", "$statements", req))
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn canonical_values_reconstruct_the_machine_and_only_history_writers_can_change_it() {
    backends(|f, url, _| {
        let (mut db, clock) = timed(f.open(url, &f.policies, true));
        let (a, b) = world(&mut db);
        let mut who = frame(a, paydirt::mint(2), &[a], "/app.MachineService/Service");
        who.trace = vec![51; 16];
        let robot = write(
            &mut db,
            &a,
            "app.Robot",
            "Add",
            json!({"tenant":owner(a),"alias":"reference"}),
        );
        let maker = paydirt::mint(9);
        let machine = write(
            &mut db,
            &a,
            "app.Machine",
            "Add",
            json!({"tenant":owner(a),"alias":"m-01","model":"X-1","hours":"9007199254740993","load":0.5,"running":true,"firmware":"AQID","cycles":"9223372036854775807","makerId":STANDARD.encode(maker),"robot":{"id":robot["id"]},"mode":3,"dateServiced":"2026-10-01T09:00:00Z","spec":{"axes":3,"reach":1.5},"tags":["b","a"],"settings":{"speed":"slow","é":"\u{2028}"},"operator":"Kim"}),
        );
        clock.store(10, Ordering::SeqCst);
        let event = transaction(&mut db, &who, |db| {
            let start = begin(db, "Machine", &machine["id"], "", "MachineStarted")?;
            op(
                db,
                "Machine",
                "$event_facts",
                json!({"event":start["id"],"after":machine}),
            )?;
            Ok(start)
        })
        .unwrap();
        assert_eq!(event["kind"], "app.MachineStarted");
        assert_eq!(
            event["value"]["@type"],
            "type.googleapis.com/app.MachineStarted"
        );
        assert_eq!(
            event["actorId"],
            STANDARD.encode(who.actor.as_ref().unwrap())
        );
        assert_eq!(event["tenant"], owner(a));
        assert_eq!(event["subjectId"], machine["id"]);
        assert_eq!(event["reason"], "due");
        assert_eq!(event["traceId"], STANDARD.encode([51; 16]));
        let state = db
            .call(
                &Scope::Tenant(a.to_vec()),
                call(
                    "Machine",
                    "$state",
                    json!({"ref":{"id":machine["id"]},"at":moment(20)}),
                ),
            )
            .unwrap();
        let slots: std::collections::BTreeMap<_, _> = state["facts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                assert_eq!(f["domain"], 19);
                (
                    (
                        f["fieldNumber"].as_u64().unwrap(),
                        f["key"].as_str().unwrap(),
                    ),
                    f["value"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let expected = [
            (4, "", json!("m-01")),
            (8, "", json!("X-1")),
            (9, "", json!(9007199254740993i64)),
            (10, "", json!(0.5)),
            (11, "", json!(true)),
            (12, "", json!("AQID")),
            (16, "", json!(i64::MAX)),
            (17, "", json!(paydirt::format_id(&maker).unwrap())),
            (
                18,
                "",
                json!(paydirt::format_id(&id(&robot["id"])).unwrap()),
            ),
            (19, "", json!(3)),
            (20, "", json!("2026-10-01T09:00:00Z")),
            (21, "", json!({"1":3,"2":1.5})),
            (22, "", json!(["b", "a"])),
            (23, "speed", json!("slow")),
            (23, "é", json!("\u{2028}")),
        ];
        assert_eq!(slots.len(), expected.len() + 1);
        for (n, k, v) in expected {
            assert_eq!(slots[&(n, k)], v.to_string(), "{n} {k}");
        }
        assert_eq!(slots[&(24, "")], "");
        let values = &state["values"];
        assert_eq!(values["id"], machine["id"]);
        assert_eq!(values["spec"], machine["spec"]);
        assert_eq!(values["mode"], "MODE_SERVICE");
        assert_eq!(values["robot"], machine["robot"]);
        assert_eq!(values["settings"], machine["settings"]);
        assert_eq!(values["cycles"], machine["cycles"]);
        assert_eq!(values["hours"], machine["hours"]);
        assert!(values.get("tenant").is_none() && values.get("operator").is_none());
        let fact = &state["facts"][0]["id"];
        for (kind, object) in [("Event", &event["id"]), ("Fact", fact)] {
            let check = db
                .verify_object(&format!("app.{kind}"), id(object))
                .unwrap();
            assert_eq!(check.digest, Verdict::Matches, "{kind}: {check:?}");
            if let Some(seal) = check.seal {
                assert_eq!(seal, Verdict::Matches);
            }
            for method in ["Add", "AddMany", "Patch", "Apply", "Erase"] {
                let request = json!({"id":object,"ref":{"id":object},"items":[{}]});
                for err in [
                    db.operator(call(kind, method, request.clone()))
                        .unwrap_err(),
                    db.call(&Scope::Internal, call(kind, method, request.clone()))
                        .unwrap_err(),
                    super::walls::invoke(&mut db, &who, call(kind, method, request.clone()))
                        .unwrap_err(),
                ] {
                    assert_eq!(err.code, "permission_denied");
                    assert_eq!(err.message, "a past is written by the past package");
                }
            }
        }
        assert_eq!(list(&mut db, a, "Event").len(), 1);
        assert_eq!(list(&mut db, a, "Fact").len(), slots.len());
        assert!(
            list(&mut db, b, "Event").is_empty()
                && list(&mut db, b, "Fact").is_empty()
                && statements(&mut db, b, json!({"subject":machine["id"]})).is_empty()
        );
        assert_eq!(
            db.call(
                &Scope::Tenant(b.to_vec()),
                call(
                    "Machine",
                    "$state",
                    json!({"ref":{"id":machine["id"]},"at":moment(20)})
                )
            )
            .unwrap_err()
            .code,
            "not_found"
        );
        let unknown = transaction(&mut db, &who, |db| {
            begin(db, "Machine", &machine["id"], "", "Machine")
        });
        assert_eq!(unknown.unwrap_err().code, "invalid_argument");
    })
}
#[test]
fn statements_correct_and_retract_standing_slots_at_their_knowledge_time() {
    backends(|f, url, _| {
        let (mut db, clock) = timed(f.open(url, &f.policies, true));
        let (a, b) = world(&mut db);
        let who = frame(a, paydirt::mint(2), &[a], "/app.History/Write");
        let mut machines = Vec::new();
        for alias in ["m1", "m2"] {
            machines.push(write(
                &mut db,
                &a,
                "app.Machine",
                "Add",
                json!({"tenant":owner(a),"alias":alias,"model":"C-2"}),
            ));
        }
        clock.store(10, Ordering::SeqCst);
        let first=transaction(&mut db,&who,|db|{
            let e=begin(db,"Machine",&machines[0]["id"],"machine.service","MachineServiced")?;
            for m in &machines{op(db,"Machine","$event_facts",json!({"event":e["id"],"dateStart":moment(5),"after":{"id":m["id"],"model":"C-2","hours":"10","mode":3}}))?;}
            Ok(e["id"].clone())
        }).unwrap();
        let originals = statements(&mut db, a, json!({"event":first}));
        assert_eq!(originals.len(), 2);
        assert_eq!(originals[0]["rows"].as_array().unwrap().len(), 3);
        let m = originals[0]["subject"].clone();
        let fact = originals[0]["rows"][1]["id"].clone();
        clock.store(20, Ordering::SeqCst);
        let correct = transaction(&mut db, &who, |db| {
            let e = begin(db, "Machine", &m, "correct", "Corrected")?;
            op(
                db,
                "Fact",
                "$correct_fact",
                json!({"event":e["id"],"target":fact,"dateStart":moment(2)}),
            )?;
            Ok(e["id"].clone())
        })
        .unwrap();
        let now = statements(&mut db, a, json!({"subject":m}));
        assert_eq!(now.len(), 2);
        assert_eq!(now[0]["event"], correct);
        assert_eq!(now[0]["rows"][0]["value"], "10");
        assert_eq!(now[1]["rows"].as_array().unwrap().len(), 2);
        let ever = statements(&mut db, a, json!({"subject":m,"ever":true}));
        assert_eq!(ever[1]["rows"].as_array().unwrap().len(), 3);
        assert_eq!(ever[1]["rows"][1]["supersededBy"], correct);
        let was = statements(&mut db, a, json!({"subject":m,"known":moment(15)}));
        assert_eq!(was.len(), 1);
        assert_eq!(was[0]["rows"].as_array().unwrap().len(), 3);
        let repeat = transaction(&mut db, &who, |db| {
            let e = begin(db, "Machine", &m, "retract", "Corrected")?;
            op(
                db,
                "Fact",
                "$correct_fact",
                json!({"event":e["id"],"target":fact}),
            )
        });
        assert_eq!(repeat.unwrap_err().code, "not_found");
        let stale = transaction(&mut db, &who, |db| {
            op(
                db,
                "Fact",
                "$correct_event",
                json!({"event":correct,"target":first}),
            )
        });
        assert_eq!(stale.unwrap_err().code, "failed_precondition");
        let outsider = frame(b, paydirt::mint(2), &[a, b], "/app.History/Write");
        let foreign = transaction(&mut db, &outsider, |db| {
            let e = begin(db, "Fact", &Value::Null, "retract", "Corrected")?;
            op(
                db,
                "Fact",
                "$correct_event",
                json!({"event":e["id"],"target":first}),
            )
        });
        assert_eq!(foreign.unwrap_err().code, "not_found");
        clock.store(30, Ordering::SeqCst);
        let second = transaction(&mut db, &who, |db| {
            let e = begin(db, "Machine", &m, "correct", "Corrected")?;
            op(
                db,
                "Fact",
                "$correct_event",
                json!({"event":e["id"],"target":first,"dateStart":moment(3)}),
            )?;
            Ok(e["id"].clone())
        })
        .unwrap();
        assert!(statements(&mut db, a, json!({"event":first})).is_empty());
        let grouped = statements(&mut db, a, json!({"event":second}));
        assert_eq!(grouped.len(), 2);
        assert!(grouped.iter().all(|s| s["dateStart"] == json!(moment(3))));
        let state = |db: &mut Store, at| {
            db.call(
                &Scope::Tenant(a.to_vec()),
                call("Machine", "$state", json!({"ref":{"id":m},"at":moment(at)})),
            )
            .unwrap()["values"]
                .clone()
        };
        assert_eq!(state(&mut db, 4)["model"], "C-2");
        assert!(state(&mut db, 1)["model"].is_null());
        clock.store(40, Ordering::SeqCst);
        transaction(&mut db, &who, |db| {
            let e = begin(db, "Machine", &m, "retract", "Corrected")?;
            op(
                db,
                "Fact",
                "$correct_event",
                json!({"event":e["id"],"target":second}),
            )
        })
        .unwrap();
        assert!(state(&mut db, 10)["model"].is_null());
        assert_eq!(state(&mut db, 10)["hours"], "10");
        for (index, valid) in [2, 8].into_iter().enumerate() {
            clock.store(50 + index as i64, Ordering::SeqCst);
            transaction(&mut db,&who,|db|{
            let e=begin(db,"Machine",&m,"machine.service","MachineServiced")?;op(db,"Machine","$event_facts",json!({"event":e["id"],"after":{"id":m,"hours":"11"},"dateStart":moment(valid)}))
        }).unwrap();
        }
        let counts = statements(&mut db, a, json!({"subject":m}));
        assert_eq!(counts.len(), 2);
        assert!(counts.iter().all(|s| s["rows"][0]["value"] == "11"));
    })
}

#[test]
fn declared_payloads_and_paged_children_preserve_one_transaction() {
    backends(|f, url, _| {
        let mut db = f.open(url, &f.policies, false);
        let (a, b) = world(&mut db);
        let who = frame(a, paydirt::mint(2), &[a], "/app.History/Batch");
        let person = STANDARD.encode(paydirt::mint(2));
        let payload = json!({"@type":"type.googleapis.com/app.MachineServiced","machine":STANDARD.encode(paydirt::mint(19)),"by":person,"mode":"MODE_SERVICE","parts":[{"serial":"P-1","count":2},{"serial":"P-2","count":1}],"swapped":{"left":{"serial":"P-0","count":1}},"dateDue":"2026-11-01T00:00:00Z"});
        let service = transaction(&mut db, &who, |db| {
            op(db, "Machine", "$begin_event", json!({"value":payload}))
        })
        .unwrap();
        assert_eq!(service["kind"], "machine.service");
        assert_eq!(service["value"], payload);
        let options = |kind: &str, ty: &str| json!({"kind":kind,"value":{"@type":format!("type.googleapis.com/app.{ty}")}});
        let parent=transaction(&mut db,&who,|db|{
            let e=begin(db,"Robot",&Value::Null,"correct","Corrected")?;
            let children=op(db,"Robot","$event_children",json!({"event":e["id"],"children":[options("retract","Corrected"),options("correct","Corrected"),options("robot.move","RobotMoved"),options("","RobotMoved")]}))?;
            assert_eq!(children.as_array().unwrap().iter().map(|v|v["kind"].as_str().unwrap()).collect::<Vec<_>>(),["retract","correct","robot.move","robot.move"]);
            assert!(children.as_array().unwrap().iter().all(|v|v["causeId"]==e["id"]));
            op(db,"Robot","$begin_event",json!({"cause":children[0]["id"],"value":{"@type":"type.googleapis.com/app.RobotMoved"}}))
        }).unwrap();
        assert!(!parent["causeId"].is_null());
        for bad in [
            options("", "Corrected"),
            options("robot.move", "Corrected"),
            options("unknown", "RobotMoved"),
            options("", "Robot"),
        ] {
            let n = list(&mut db, a, "Event").len();
            let run = transaction(&mut db, &who, |db| {
                let e = begin(db, "Robot", &Value::Null, "correct", "Corrected")?;
                op(
                    db,
                    "Robot",
                    "$event_children",
                    json!({"event":e["id"],"children":[options("","RobotMoved"),bad]}),
                )
            });
            assert!(run.unwrap_err().message.contains("child 1:"));
            assert_eq!(list(&mut db, a, "Event").len(), n);
        }
        let domain = f.policies["app.Robot"]["entity"]["domain"]
            .as_u64()
            .unwrap() as u8;
        let robot_ids: Vec<_> = (0..1500)
            .map(|_| STANDARD.encode(paydirt::mint(domain)))
            .collect();
        for (p, page) in robot_ids.chunks(500).enumerate() {
            let items:Vec<_>=page.iter().enumerate().map(|(i,id)|json!({"id":id,"tenant":owner(a),"alias":format!("child-{}",500*p+i)})).collect();
            db.call(
                &Scope::Internal,
                call("Robot", "AddMany", json!({"items":items})),
            )
            .unwrap();
        }
        let mut children:Vec<_>=robot_ids.iter().map(|id|json!({"entity":"app.Robot","subject":id,"value":{"@type":"type.googleapis.com/app.RobotMoved"},"after":{"id":id,"alias":"moved","keeper":"private"}})).collect();
        let foreign = write(
            &mut db,
            &b,
            "app.Robot",
            "Add",
            json!({"tenant":owner(b),"alias":"elsewhere"}),
        );
        let last = children.last().unwrap().clone();
        children.last_mut().unwrap()["subject"] = foreign["id"].clone();
        let batch = |db: &mut Store, children: &[Value]| {
            transaction(db, &who, |db| {
                let e = begin(db, "Robot", &Value::Null, "", "RobotMoved")?;
                op(
                    db,
                    "Robot",
                    "$event_children",
                    json!({"event":e["id"],"children":children}),
                )
            })
        };
        assert_eq!(batch(&mut db, &children).unwrap_err().code, "not_found");
        assert!(statements(&mut db, a, json!({})).is_empty());
        *children.last_mut().unwrap() = last;
        let rows = batch(&mut db, &children).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1500);
        let facts = statements(&mut db, a, json!({}));
        assert_eq!(facts.len(), 1500);
        assert!(
            facts
                .iter()
                .all(|s| s["rows"].as_array().unwrap().len() == 2
                    && s["rows"][0]["value"] == "\"moved\""
                    && s["rows"][1]["value"] == "")
        );
    })
}
#[test]
fn expiry_cleans_application_pins_and_records_only_counts_in_the_tenants_transaction() {
    backends(|f, url, _| {
        let (mut db, clock) = timed(f.open(url, &f.policies, true));
        let (a, b) = world(&mut db);
        let mut pinned = Vec::new();
        for t in [a, b] {
            let who = frame(t, paydirt::mint(2), &[t], "/app.History/Write");
            let robot = write(
                &mut db,
                &t,
                "app.Robot",
                "Add",
                json!({"tenant":owner(t),"alias":"stint"}),
            );
            for i in 0..3 {
                clock.store(i, Ordering::SeqCst);
                let e=transaction(&mut db,&who,|db|{
                    let e=begin(db,"Robot",&robot["id"],"","RobotMoved")?;
                    op(db,"Robot","$event_facts",json!({"event":e["id"],"after":{"id":robot["id"],"alias":format!("state-{i}")},"dateStart":moment(i)}))?;
                    Ok(e)
                }).unwrap();
                if i == 0 {
                    let stint = write(
                        &mut db,
                        &t,
                        "app.Stint",
                        "Add",
                        json!({"eventId":e["id"],"robotId":robot["id"],"tenant":owner(t),"dateEnd":moment(5)}),
                    );
                    pinned.push((stint, e));
                }
            }
        }
        clock.store(100, Ordering::SeqCst);
        let w = Window {
            keep_ms: 10_000,
            ..Default::default()
        };
        let preview = db.expire_past(a, &w, true).unwrap();
        assert_eq!(preview.facts.len(), 2);
        assert_eq!(preview.events.len(), 1);
        let cleanup = |tx: &mut paydirt_store::Transaction<'_>, cut| {
            assert_eq!(cut, moment(90));
            tx.call(call("Stint", "Erase", json!({"id":pinned[0].0["id"]})))?;
            Ok(1)
        };
        let plan = db.expire_past_with(a, &w, true, cleanup).unwrap();
        assert_eq!(plan.events.len(), 2);
        assert_eq!(plan.stints, 1);
        assert!(
            db.call(
                &Scope::Tenant(a.to_vec()),
                call("Stint", "Get", json!({"ref":{"id":pinned[0].0["id"]}}))
            )
            .is_ok()
        );
        let failed = db.expire_past_with(a, &w, false, |tx, cut| {
            cleanup(tx, cut)?;
            Err(Fault::precondition("stint policy failed"))
        });
        assert!(failed.is_err());
        assert_eq!(list(&mut db, a, "Event").len(), 3);
        let done = db.expire_past_with(a, &w, false, cleanup).unwrap();
        assert_eq!(done.events, plan.events);
        assert_eq!(done.facts, plan.facts);
        assert_eq!(
            db.call(
                &Scope::Tenant(a.to_vec()),
                call("Stint", "Get", json!({"ref":{"id":pinned[0].0["id"]}}))
            )
            .unwrap_err()
            .code,
            "not_found"
        );
        let events = list(&mut db, a, "Event");
        let receipt = events.iter().find(|e| e["kind"] == "past.expire").unwrap();
        assert!(receipt["actorId"].is_null() && receipt["subjectId"].is_null());
        assert_eq!(receipt["tenant"], owner(a));
        assert_eq!(receipt["reason"], "");
        assert_eq!(
            receipt["value"],
            json!({"@type":"type.googleapis.com/app.PastExpired","dateBefore":moment(90),"facts":2,"events":2,"stints":1})
        );
        let lines = list(&mut db, a, "Audit");
        for taken in done
            .facts
            .iter()
            .chain(&done.events)
            .chain(pinned[0].0["id"].as_str().map(str::to_string).iter())
        {
            let matches: Vec<_> = lines
                .iter()
                .filter(|l| l["objectId"] == *taken && l["action"] == "payday.past.expire")
                .collect();
            assert_eq!(matches.len(), 1);
            assert_eq!(matches[0]["actorId"], STANDARD.encode([0; 16]));
            assert_eq!(matches[0]["tenantId"], STANDARD.encode(a));
        }
        let again = db.expire_past(a, &w, false).unwrap();
        assert!(again.events.is_empty() && again.facts.is_empty());
        assert_eq!(list(&mut db, a, "Event").len(), 2);
        assert_eq!(
            db.expire_past(paydirt::mint(1), &w, false)
                .err()
                .unwrap()
                .code,
            "not_found"
        );
        assert_eq!(
            db.call(
                &Scope::Internal,
                call("Event", "$expire_past", json!({"window":w}))
            )
            .unwrap_err()
            .code,
            "failed_precondition"
        );
        let outsider = db.transaction(&Scope::Tenant(b.to_vec()), |tx| {
            tx.call(call("Event", "$expire_past", json!({"window":w})))
        });
        assert!(outsider.is_err());
        let results = db
            .expire_all_past(|t| {
                if t == a {
                    Err(Fault::precondition("cannot read window"))
                } else {
                    Ok(w.clone())
                }
            })
            .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|(t, r)| *t == a && r.is_err()));
        assert!(
            results
                .iter()
                .any(|(t, r)| *t == b && r.as_ref().unwrap().facts.len() == 2)
        );
        assert_eq!(list(&mut db, a, "Event").len(), 2);
    })
}
#[test]
fn a_second_expirer_waits_for_the_same_tenants_cleanup_and_records_each_removal_once() {
    use std::sync::mpsc;
    use std::time::Duration;
    backends(|f, url, _| {
        if !url.starts_with("postgres") {
            return;
        }
        let (mut setup, clock) = timed(f.open(url, &f.policies, true));
        let a = provision(&mut setup);
        let who = frame(a, paydirt::mint(2), &[a], "/app.History/Add");
        let row = transaction(&mut setup, &who, |db| {
            begin(db, "Robot", &Value::Null, "", "RobotMoved")
        })
        .unwrap();
        clock.store(100, Ordering::SeqCst);
        let (locked, acquired) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let (ready, started) = mpsc::channel();
        let (done, result) = mpsc::channel();
        std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                let mut store = f
                    .open(url, &f.policies, true)
                    .with_inputs(Inputs::new(|| moment(100), paydirt::mint));
                store
                    .expire_past_with(
                        a,
                        &Window {
                            keep_ms: 10_000,
                            ..Default::default()
                        },
                        false,
                        move |_, _| {
                            locked.send(()).unwrap();
                            released.recv_timeout(Duration::from_secs(20)).unwrap();
                            Ok(0)
                        },
                    )
                    .unwrap()
            });
            acquired.recv_timeout(Duration::from_secs(10)).unwrap();
            scope.spawn(|| {
                let mut store = f
                    .open(url, &f.policies, true)
                    .with_inputs(Inputs::new(|| moment(100), paydirt::mint));
                ready.send(()).unwrap();
                done.send(
                    store
                        .expire_past(
                            a,
                            &Window {
                                keep_ms: 10_000,
                                ..Default::default()
                            },
                            false,
                        )
                        .unwrap(),
                )
                .unwrap();
            });
            started.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(result.recv_timeout(Duration::from_millis(120)).is_err());
            release.send(()).unwrap();
            assert_eq!(
                first.join().unwrap().events,
                vec![row["id"].as_str().unwrap().to_string()]
            );
            let second = result.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(second.events.is_empty() && second.facts.is_empty());
        });
        let audit = list(&mut setup, a, "Audit");
        assert_eq!(
            audit
                .iter()
                .filter(|r| r["action"] == "payday.past.expire" && r["objectId"] == row["id"])
                .count(),
            1
        );
    })
}
