mod operator;
use __APP_CRATE__::{
    router,
    transport::{Database, MODEL, POLICIES},
};
use connectrpc::Server;
use serde::Deserialize;
use std::{error::Error, fs};

#[derive(Deserialize)]
struct Config {
    listen: std::net::SocketAddr,
    service_name: String,
    #[serde(default)]
    allow_general_writes: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "paydirt.sqlite3".into());
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        // Preserve the host-only provisioning command. Ordinary lowercase verbs use the deployed API.
        if !matches!(args.as_slice(),[entity,method,_] if entity.contains('.') && method.as_bytes().first().is_some_and(u8::is_ascii_uppercase))
        {
            return operator::run(&args).await;
        }
        let [entity, method, argument] = args.as_slice() else {
            return Err("usage: APP ENTITY METHOD JSON".into());
        };
        let mut store = paydirt_store::Store::open(&url, MODEL, POLICIES)?;
        let call = paydirt_store::Call {
            entity: entity.clone(),
            method: method.clone(),
            request: serde_json::from_str(argument)?,
        };
        println!("{}", store.call(&paydirt_store::Scope::Internal, call)?);
        return Ok(());
    }
    let config: Config = serde_json::from_slice(&fs::read("app.json")?)?;
    let addr = match std::env::var("PAYDIRT_LISTEN") {
        Ok(addr) => addr.parse()?,
        Err(std::env::VarError::NotPresent) => config.listen,
        Err(e) => return Err(e.into()),
    };
    let principals = std::env::var("PAYDIRT_PRINCIPALS").unwrap_or_else(|_| "{}".into());
    let db = Database::open(&url, &principals)?.allow_general_writes(config.allow_general_writes);
    Server::new(router(db, config.service_name))
        .serve(addr)
        .await
        .map_err(|e| e as Box<dyn Error>)
}
