#![cfg(any(feature = "postgres", feature = "sqlite"))]
use paydirt_store::{Call, Scope, Store};
use serde_json::{Value, json};

fn exercise(url: &str) {
    let pk = json!({"name":"id","json":"id","number":1,"kind":"INT32","immutable":true});
    let model = json!({"version":1,"entities":[
        {"name":"sample.Group","key":"id","indexes":[],"fields":[pk,{"name":"people","json":"people","kind":"MESSAGE","number":2,"collection":"list","target":"sample.Person"}]},
        {"name":"sample.Person","key":"id","indexes":[],"fields":[pk,{"name":"groups","json":"groups","kind":"MESSAGE","number":2,"collection":"list","target":"sample.Group","inverse":"people"},
            {"name":"tags","json":"tags","kind":"STRING","collection":"list","number":3},{"name":"labels","json":"labels","kind":"JSON","collection":"map","number":4},
            {"name":"nick","json":"nick","kind":"STRING","nullable":true,"number":5}]}
    ]});
    let mut db = Store::open(url, &model.to_string(), "{}").unwrap();
    let mut call = |name: &str, method: &str, request: Value| {
        db.call(
            &Scope::Internal,
            Call {
                entity: format!("sample.{name}"),
                method: method.into(),
                request,
            },
        )
    };
    for id in [1, 2] {
        call(
            "Person",
            "Add",
            json!({"id":id,"tags":["first","second"],"labels":{"region":"north"}}),
        )
        .unwrap();
    }
    let team = call("Group", "Add", json!({"id":7,"people":[{"id":1},{"id":2}]})).unwrap();
    assert_eq!(team["people"], json!([{"id":1},{"id":2}]));
    let get = || json!({"ref":{"id":1}});
    assert_eq!(
        call("Person", "Get", get()).unwrap()["groups"],
        json!([{"id":7}])
    );
    let patch = |entries: Value| json!({"ref":{"id":1},"patch":{"messageType":"sample.Person","delta":{"entries":entries}}});
    let field = |n| json!({"key":{"field":{"number":n}}});
    let assign = |s: &str| json!({"targets":{"selectors":[field(5)]},"assign":{"value":{"s":s}}});
    let condition = |exists| json!({"targets":{"selectors":[field(5)]},"test":{"exists":exists}});
    let result = call(
        "Person",
        "Apply",
        patch(json!([condition(false), assign("")])),
    )
    .unwrap();
    assert_eq!(result["nick"], "");
    assert_eq!(
        call(
            "Person",
            "Apply",
            patch(json!([condition(false), assign("lost")]))
        )
        .unwrap_err()
        .code,
        "failed_precondition"
    );
    let map = json!({"path":{"segments":[{"field":{"name":"labels"}}]},"targets":{"selectors":[{"key":{"mapKey":{"s":"region"}}}]},"assign":{"value":{"s":"east"}}});
    let list = json!({"path":{"segments":[{"field":{"name":"tags"}}]},"targets":{"selectors":[{"key":{"index":"0"}}]},"remove":{}});
    let result = call("Person", "Apply", patch(json!([map, list]))).unwrap();
    assert_eq!(result["labels"]["region"], "east");
    assert_eq!(result["tags"], json!(["second"]));
    assert_eq!(result["nick"], "");
    assert_eq!(
        call(
            "Group",
            "Patch",
            json!({"ref":{"id":7},"people":[{"id":2}]})
        )
        .unwrap()["people"],
        json!([{"id":2}])
    );
    assert_eq!(call("Person", "Get", get()).unwrap()["groups"], json!([]));
    assert_eq!(
        call("Person", "Erase", json!({"id":2})).unwrap()["erased"],
        true
    );
    assert_eq!(
        call("Group", "Get", json!({"ref":{"id":7}})).unwrap()["people"],
        json!([])
    );
}
#[cfg(feature = "sqlite")]
#[test]
fn sqlite() {
    exercise(":memory:")
}
#[cfg(feature = "postgres")]
#[test]
fn postgres() {
    use diesel::{Connection as _, connection::SimpleConnection};
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let schema = format!("pd_patch_{}", hex::encode(paydirt::mint(255)));
    let mut c = diesel::pg::PgConnection::establish(&url).unwrap();
    c.batch_execute(&format!("CREATE SCHEMA {schema}")).unwrap();
    let separator = if url.contains('?') { "&" } else { "?" };
    let result = std::panic::catch_unwind(|| {
        exercise(&format!("{url}{separator}options=-csearch_path%3D{schema}"))
    });
    c.batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .unwrap();
    if let Err(e) = result {
        std::panic::resume_unwind(e)
    }
}
