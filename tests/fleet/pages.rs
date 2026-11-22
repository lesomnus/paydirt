#![cfg(feature = "client")]
//! Exercise the generated RPC over TCP as well as the same SQLite store directly.
use base64::{Engine, engine::general_purpose::STANDARD};
use connectrpc::client::{ClientConfig, HttpClient};
use fleet::{
    proto::app::RobotListRequest,
    router,
    services::app::RobotServiceClient,
    transport::{Database, MODEL, POLICIES},
};
use futures_util::{StreamExt, TryStreamExt};
use paydirt_store::{Call, Scope};
use serde_json::json;

#[tokio::test]
async fn direct_and_network_pages_return_all_rows_and_preserve_the_original_cursor() {
    let fixture = paydirt_test::Fixture::new().unwrap();
    let mut store = fixture.store(MODEL, POLICIES).unwrap();
    let tenant = paydirt_test::id(1, 300);
    store
        .seed(vec![(
            "app.Tenant".into(),
            json!({"id":STANDARD.encode(tenant),"alias":"acme"}),
        )])
        .unwrap();
    for alias in ["d", "b", "a", "e", "c"] {
        store
            .call(
                &Scope::Tenant(tenant.to_vec()),
                Call {
                    entity: "app.Robot".into(),
                    method: "Add".into(),
                    request: json!({"tenant":{"id":STANDARD.encode(tenant)},"alias":alias}),
                },
            )
            .unwrap();
    }
    let mut after = String::new();
    let mut direct = Vec::new();
    loop {
        let page = store
            .call(
                &Scope::Tenant(tenant.to_vec()),
                Call {
                    entity: "app.Robot".into(),
                    method: "List".into(),
                    request: json!({"size":2,"order":1,"after":after}),
                },
            )
            .unwrap();
        direct.extend(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|r| r["alias"].as_str().map(str::to_owned)),
        );
        after = page["next"].as_str().unwrap_or("").into();
        if after.is_empty() {
            break;
        }
    }
    let db = Database::from_store(
        store,
        &json!({"member":tenant.iter().map(|b|format!("{b:02x}")).collect::<String>()}).to_string(),
    )
    .unwrap();
    let server = connectrpc::Server::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", server.local_addr().unwrap());
    let task = tokio::spawn(server.serve(router(db, "pages".into())));
    let config = ClientConfig::new(endpoint.parse().unwrap()).with_default_header(
        "authorization",
        http::HeaderValue::from_static("Bearer member"),
    );
    let rpc = RobotServiceClient::new(HttpClient::plaintext(), config);
    let request: RobotListRequest = serde_json::from_value(json!({"size":2,"order":1})).unwrap();
    let rows = paydirt::page::all(
        &request,
        |q| &mut q.after,
        |q| async {
            let reply = rpc.list(q).await?.into_owned();
            Ok((reply.items, reply.next))
        },
        || connectrpc::ConnectError::internal("repeated page"),
    );
    let wire: Vec<_> = rows
        .map(|r| r.map(|v| v.alias))
        .try_collect()
        .await
        .unwrap();
    assert_eq!(direct, ["a", "b", "c", "d", "e"]);
    assert_eq!(wire, direct);
    assert!(request.after.is_empty());
    task.abort();
    let _ = task.await;
}
