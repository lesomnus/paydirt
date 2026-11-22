use paydirt_server::axum::{Router, http::StatusCode, routing::get};
pub fn http(_: &fleet::transport::Database) -> Router {
    Router::new()
        .route("/login", get(|| async { "token" }))
        .route("/teapot", get(|| async { StatusCode::IM_A_TEAPOT }))
}

pub fn policy(
    db: fleet::transport::Database,
) -> Result<fleet::transport::Database, Box<dyn std::error::Error>> {
    let Ok(config) = std::env::var("FLEET_CONSOLE_TENANTS") else {
        return Ok(db);
    };
    let owners: Vec<_> = config
        .split(',')
        .map(|s| paydirt::parse_id(s).ok_or("invalid console tenant"))
        .collect::<Result<_, _>>()?;
    Ok(db.with_tenant_policy(move |_, method| {
        if !method.ends_with("/List") && !method.ends_with("/Get") {
            return Err(connectrpc::ConnectError::permission_denied(
                "the console reads and does not write",
            ));
        }
        Ok(Some(owners.clone()))
    }))
}
