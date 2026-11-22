//! Pinned SDK test server. Only this localhost fixture accepts Payday's Plain sign-in form.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use fleet::{
    router,
    transport::{Database, MODEL, POLICIES, error},
};
use paydirt_store::{Store, auth::Sessions};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Duration};

#[derive(Clone)]
struct Issuer {
    sessions: Sessions,
    holders: Arc<HashMap<String, String>>,
}
fn cookie(h: &HeaderMap) -> Option<&str> {
    h.get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|s| s.strip_prefix("paydirt-session="))
}
async fn signin(State(s): State<Issuer>, Json(body): Json<Value>) -> impl IntoResponse {
    let Some(id) = body["Id"]
        .as_str()
        .and_then(|id| s.holders.get_key_value(id))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let key = s
        .sessions
        .issue(id.1.clone(), id.0.clone(), Duration::from_secs(3600))
        .unwrap();
    (
        StatusCode::NO_CONTENT,
        [(
            "set-cookie",
            format!("paydirt-session={key}; HttpOnly; Path=/; SameSite=Lax"),
        )],
    )
        .into_response()
}
async fn which(State(s): State<Issuer>, h: HeaderMap) -> impl IntoResponse {
    let session = cookie(&h).and_then(|k| s.sessions.read(k, []).ok());
    match session {
        Some(v) => Json(json!({"identity":v.identity})).into_response(),
        None => StatusCode::UNAUTHORIZED.into_response(),
    }
}
async fn end(State(s): State<Issuer>, h: HeaderMap) -> impl IntoResponse {
    if let Some(k) = cookie(&h) {
        s.sessions.revoke(k);
    }
    (
        StatusCode::NO_CONTENT,
        [("set-cookie", "paydirt-session=; Max-Age=0; Path=/")],
    )
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("sdk.db");
    let url = path.to_str().unwrap();
    let t = paydirt::mint(1);
    let tenant = STANDARD.encode(t);
    let mut rows = vec![("app.Tenant".into(), json!({"id":tenant,"alias":"acme"}))];
    let mut holders = HashMap::new();
    let mut principals = json!({});
    for who in ["admin", "bob"] {
        let id = paydirt::mint(2);
        holders.insert(paydirt::format_id(&id).unwrap(), who.to_string());
        principals[who] = json!({"tenant":hex::encode(t),"actor":hex::encode(id)});
        rows.push((
            "app.Holder".into(),
            json!({"id":STANDARD.encode(id),"alias":who,"tenant":{"id":tenant}}),
        ));
    }
    Store::open(url, MODEL, POLICIES)?.seed(rows)?;
    let issuer = Issuer {
        sessions: Default::default(),
        holders: Arc::new(holders),
    };
    let sessions = issuer.sessions.clone();
    let db = Database::open(url, &principals.to_string())?
        .allow_general_writes(true)
        .with_resolver(move |ctx| {
            let h = ctx.headers();
            if let Some(key) = cookie(h) {
                let expected = h
                    .get_all("payday-identity")
                    .iter()
                    .map(|v| v.to_str().unwrap_or(""));
                return sessions
                    .read(key, expected)
                    .map(|s| Some(s.principal))
                    .map_err(error);
            }
            Ok(
                (h.get("authorization").and_then(|v| v.to_str().ok()) == Some("Plain @acme/admin"))
                    .then(|| "admin".to_owned()),
            )
        });
    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let rpc = connectrpc::Server::bind("127.0.0.1:0").await?;
    println!("{}\n{}", rpc.local_addr()?, http.local_addr()?);
    let app = Router::new()
        .route("/session", get(which).post(signin).delete(end))
        .with_state(issuer);
    tokio::select! {v=rpc.serve(router(db,"sdk".into()))=>v?,v=axum::serve(http,app)=>v?};
    Ok(())
}
