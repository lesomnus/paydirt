mod operator;
use __APP_CRATE__::{
    router,
    transport::{Database, MODEL, POLICIES},
};
use connectrpc::Server;
use paydirt::watch::{Broker, Memory, nats::Nats, postgres::Postgres};
use serde::Deserialize;
use std::{error::Error, fs, sync::Arc, time::Duration};

#[derive(Deserialize)]
struct Watch {
    broker: String,
    #[serde(default)]
    dsn: String,
    #[serde(default)]
    namespace: String,
    #[serde(default)]
    outbox: bool,
    drain_every_ms: Option<u64>,
}
impl Watch {
    async fn connect(&self, url: &str) -> Result<Option<Arc<dyn Broker>>, Box<dyn Error>> {
        if self.drain_every_ms.is_some()
            && (!self.outbox || self.drain_every_ms == Some(0) || self.broker == "none")
        {
            return Err("draining requires outbox, a broker and a positive interval".into());
        }
        Ok(match self.broker.as_str() {
            "memory" => Some(Arc::new(Memory::default())),
            "postgres" => Some(Arc::new(Postgres::new(if self.dsn.is_empty() {
                url
            } else {
                &self.dsn
            }))),
            "nats" => Some(Arc::new(Nats::connect(&self.dsn, &self.namespace).await?)),
            "none" => None,
            name => return Err(format!("unknown watch broker {name}").into()),
        })
    }
}
#[derive(Deserialize)]
struct Config {
    watch: Watch,
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
        let config: Config = serde_json::from_slice(&fs::read("app.json")?)?;
        let broker = config.watch.connect(&url).await?;
        let mut store =
            paydirt_store::Store::open(&url, MODEL, POLICIES)?.with_outbox(config.watch.outbox);
        let call = paydirt_store::Call {
            entity: entity.clone(),
            method: method.clone(),
            request: serde_json::from_str(argument)?,
        };
        let result = store.operator(call)?;
        if let (Some(event), Some(broker)) = (store.take_committed(), broker) {
            if let Err(e) = broker.publish(event).await {
                eprintln!("watch publication failed: {e}");
            }
        }
        println!("{}", result);
        return Ok(());
    }
    let config: Config = serde_json::from_slice(&fs::read("app.json")?)?;
    let addr = match std::env::var("PAYDIRT_LISTEN") {
        Ok(addr) => addr.parse()?,
        Err(std::env::VarError::NotPresent) => config.listen,
        Err(e) => return Err(e.into()),
    };
    let principals = std::env::var("PAYDIRT_PRINCIPALS").unwrap_or_else(|_| "{}".into());
    let broker = config.watch.connect(&url).await?;
    let store = paydirt_store::Store::open(&url, MODEL, POLICIES)?.with_outbox(config.watch.outbox);
    let mut db = Database::from_store(store, &principals)?
        .with_broker(broker.clone())
        .allow_general_writes(config.allow_general_writes);
    if let Ok(path) = std::env::var("PAYDIRT_POLICY_FILE") {
        let refresh = std::env::var("PAYDIRT_POLICY_REFRESH_MS")
            .ok()
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(5000);
        db = db.with_policy(paydirt::rbac::Policy::file(
            path.into(),
            std::time::Duration::from_millis(refresh),
        )?);
    }
    if let Ok(header) = std::env::var("PAYDIRT_ACCOUNT_HEADER") {
        db = db.with_account_header(header)?;
    }
    if let (Some(ms), Some(broker)) = (config.watch.drain_every_ms, broker) {
        let mut store = paydirt_store::Store::open(&url, MODEL, POLICIES)?;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(ms));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                if let Err(e) = store.drain_once(&*broker, 128).await {
                    eprintln!("outbox drain failed: {e}");
                }
            }
        });
    }
    Server::new(router(db, config.service_name))
        .serve(addr)
        .await
        .map_err(|e| e as Box<dyn Error>)
}
