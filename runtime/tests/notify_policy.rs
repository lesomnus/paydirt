#![cfg(any(feature = "sqlite", feature = "postgres"))]
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use paydirt_store::{
    Call, Fault, Frame, Inputs, Packet, Result, Scope, Store, TopicGrants,
    notify::{Policy, Rows},
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

fn cell_id(t: &[u8]) -> String {
    let mut id = t.to_vec();
    id[9] = 10;
    STANDARD.encode(id)
}
struct Audience;
impl Policy for Audience {
    fn resolve(
        &self,
        rows: &mut Rows<'_, '_>,
        caller: &Frame,
        tenant: &[u8],
        method: &str,
        request: &Value,
    ) -> Result<(Vec<u8>, TopicGrants)> {
        if request.get("kind").and_then(Value::as_str) == Some("backend-down") {
            return Err(Fault::unavailable("policy dependency"));
        }
        let row = rows.get("app.Cell", &json!({"id":cell_id(tenant)}))?;
        if row.is_none() {
            return Err(Fault::missing());
        }
        let reader = caller.actor.clone().unwrap();
        Ok((
            reader,
            TopicGrants {
                read: vec!["n:alerts".into()],
                publish: if method == "Add" {
                    vec!["n:alerts".into()]
                } else {
                    vec![]
                },
                producer: Some("server".into()),
            },
        ))
    }
}
fn call(entity: &str, method: &str, request: Value) -> Call {
    Call {
        entity: entity.into(),
        method: method.into(),
        request,
    }
}
fn exercise(url: &str, project: &Path) {
    let schema =
        |name: &str| std::fs::read_to_string(project.join("generated/storage").join(name)).unwrap();
    let clock = Arc::new(AtomicI64::new(1_700_000_000));
    let now = clock.clone();
    let mut db = Store::open(url, &schema("model.json"), &schema("policies.json"))
        .unwrap()
        .with_inputs(Inputs::new(
            move || DateTime::<Utc>::from_timestamp(now.load(Ordering::Relaxed), 0).unwrap(),
            paydirt::mint,
        ))
        .with_notification_policy(Audience);
    let tenant = paydirt::mint(1);
    let other = paydirt::mint(1);
    let actor = paydirt::mint(2).to_vec();
    for t in [tenant, other] {
        db.seed(vec![(
            "app.Tenant".into(),
            json!({"id":STANDARD.encode(t),"alias":format!("topic-{}",hex::encode(t))}),
        )])
        .unwrap();
    }
    let begin = |db: &mut Store, t: [u8; 16]| {
        db.handle(Packet::Begin {
            id: "op".into(),
            frame: Frame {
                tenant: t.to_vec(),
                actor: Some(actor.clone()),
                action: "/app.Example/Send".into(),
                trace: paydirt::mint(99).to_vec(),
                ..serde_json::from_value(json!({"action":"","tenant":[]})).unwrap()
            },
        })
    };
    let run = |db: &mut Store, c: Call| {
        db.handle(Packet::Call {
            id: "op".into(),
            call: c,
        })
    };
    let finish = |db: &mut Store, commit| {
        db.handle(Packet::Finish {
            id: "op".into(),
            commit,
        })
    };
    let publish = |t: [u8; 16], kind: &str| {
        call(
            "app.Notification",
            "Add",
            json!({"tenantId":STANDARD.encode(t),"topic":{"name":"alerts"},"kind":kind,"idempotencyKey":"stable"}),
        )
    };
    let mut ids = vec![];
    let mut receipts = vec![];
    for t in [tenant, other] {
        begin(&mut db, t).unwrap();
        assert_eq!(
            run(&mut db, publish(t, "new")).unwrap_err().code,
            "not_found"
        );
        finish(&mut db, false).unwrap();
        begin(&mut db, t).unwrap();
        run(
            &mut db,
            call(
                "app.Cell",
                "Add",
                json!({"id":cell_id(&t),"tenant":{"id":STANDARD.encode(t)},"alias":"recipient"}),
            ),
        )
        .unwrap();
        let n = run(&mut db, publish(t, "new")).unwrap();
        let repeated = run(&mut db, publish(t, "new")).unwrap();
        assert_eq!(n["id"], repeated["id"]);
        let receipt = run(
            &mut db,
            call(
                "app.Notification",
                "Mark",
                json!({"tenantId":STANDARD.encode(t),"refs":[{"id":n["id"]}]}),
            ),
        )
        .unwrap();
        finish(&mut db, true).unwrap();
        ids.push(n["id"].clone());
        receipts.push(receipt["items"][0]["id"].clone());
    }
    begin(&mut db, tenant).unwrap();
    assert_eq!(
        run(&mut db, publish(tenant, "backend-down"))
            .unwrap_err()
            .code,
        "unavailable"
    );
    finish(&mut db, false).unwrap();
    clock.fetch_add(90 * 86400, Ordering::Relaxed);
    assert_eq!(db.expire_notifications(Some(tenant), 1).unwrap(), 0);
    clock.fetch_add(1, Ordering::Relaxed);
    assert_eq!(db.expire_notifications(Some(tenant), 1).unwrap(), 1);
    assert!(
        db.take_committed()
            .unwrap()
            .changes
            .iter()
            .any(|c| !c.signal.is_empty())
    );
    assert_eq!(db.expire_notifications(Some(tenant), 1).unwrap(), 0);
    for (t, index) in [(other, 1), (tenant, 0)] {
        begin(&mut db, t).unwrap();
        let read = run(
            &mut db,
            call(
                "app.Notification",
                "Read",
                json!({"tenantId":STANDARD.encode(t),"topics":[{"name":"alerts"}]}),
            ),
        )
        .unwrap();
        if t == other {
            assert_eq!(read["items"][0]["receipt"]["notificationId"], ids[index]);
        } else {
            assert_eq!(read["items"], json!([]));
            let again = run(&mut db, publish(t, "new")).unwrap();
            assert_ne!(again["id"], ids[index])
        }
        finish(&mut db, true).unwrap();
    }
    let get_receipt = |id: &Value| {
        call(
            "app.NotificationReceiptRecord",
            "Get",
            json!({"ref":{"id":id}}),
        )
    };
    assert_eq!(
        db.call(&Scope::Internal, get_receipt(&receipts[0]))
            .unwrap_err()
            .code,
        "not_found"
    );
    assert_eq!(
        db.call(&Scope::Internal, get_receipt(&receipts[1]))
            .unwrap()["notificationId"],
        ids[1]
    );
}
#[test]
fn generated_notification_schema() {
    if let Some(project) = std::env::var_os("CELL_PROJECT") {
        #[cfg(feature = "sqlite")]
        exercise(":memory:", Path::new(&project));
        #[cfg(feature = "postgres")]
        if let Ok(url) = std::env::var("DATABASE_URL") {
            use diesel::{Connection, connection::SimpleConnection};
            let schema = format!("notify_{}", hex::encode(paydirt::mint(1)));
            let mut admin = diesel::PgConnection::establish(&url).unwrap();
            admin
                .batch_execute(&format!("CREATE SCHEMA {schema}"))
                .unwrap();
            let scoped = format!(
                "{url}{}options=-csearch_path%3D{schema}",
                if url.contains('?') { "&" } else { "?" }
            );
            let test = std::panic::catch_unwind(|| exercise(&scoped, Path::new(&project)));
            admin
                .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
                .unwrap();
            if let Err(e) = test {
                std::panic::resume_unwind(e)
            }
        }
    }
}
