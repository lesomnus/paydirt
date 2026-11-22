#![cfg(not(target_arch = "wasm32"))]
use __APP_CRATE__::{
    proto::app::{Setting, SettingGetRequest},
    router,
    transport::{Database, MODEL, POLICIES},
};
use connectrpc::{ErrorCode, client::call_unary};
use paydirt_test::{Fixture, assert_code, client};
use serde_json::json;

#[tokio::test]
async fn application_router_handles_reads_and_errors() {
    let fixture = Fixture::new().unwrap();
    let mut store = fixture.store(MODEL, POLICIES).unwrap();
    let tenant = store
        .operator(paydirt_store::Call {
            entity: "app.Tenant".into(),
            method: "Add".into(),
            request: json!({"alias":"sample"}),
        })
        .unwrap();
    use base64::{Engine, engine::general_purpose::STANDARD};
    assert_eq!(tenant["id"], STANDARD.encode(paydirt_test::id(1, 1)));
    assert_eq!(
        tenant["dateUpdated"]
            .as_str()
            .unwrap()
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap(),
        paydirt_test::clock()
    );
    let db = Database::from_store(store, "{}").unwrap();
    let (http, config) = client(router(db, "test deployment".into()));
    let get = |alias: &str| {
        call_unary::<_, SettingGetRequest, <Setting as buffa::HasMessageView>::View<'static>>(
            &http,
            &config,
            "app.SettingService",
            "Get",
            serde_json::from_value(json!({"ref":{"alias":alias}})).unwrap(),
            Default::default(),
        )
    };
    assert_eq!(
        get("service").await.unwrap().into_owned().value,
        "test deployment"
    );
    assert_code(get("absent").await, ErrorCode::NotFound);
}
