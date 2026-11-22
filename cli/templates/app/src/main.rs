mod commands;
mod grants;
mod operator;
mod tree;
use __APP_CRATE__::{
    router,
    transport::{Database, MODEL, POLICIES},
};

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
struct Meter {
    directory: std::path::PathBuf,
    every_ms: Option<u64>,
    retry_ms: Option<u64>,
}
#[derive(Deserialize)]
struct Config {
    watch: Watch,
    authentication: Option<paydirt_server::authentication::Config>,
    #[serde(default)]
    serving: paydirt_server::Serving,
    rate: Option<paydirt_server::Rate>,
    meter: Option<Meter>,
    notification_expiry_ms: Option<u64>,

    grant_source: Option<grants::Source>,
    #[serde(default)]
    worker_lease: Option<String>,
    listen: std::net::SocketAddr,
    service_name: String,
    #[serde(default)]
    allow_general_writes: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "paydirt.sqlite3".into());
    let args: Vec<_> = std::env::args().skip(1).collect();
    if let [command, tenant, config]=args.as_slice() && command=="past-expire" {
        #[derive(Deserialize)]
        struct Expiry {window:paydirt_store::past_expiry::Window,#[serde(default)]dry_run:bool}
        let req:Expiry=serde_json::from_str(config)?;
        let mut store=__APP_CRATE__::resource::record(paydirt_store::Store::open(&url,MODEL,POLICIES)?)?;
        println!("{}",serde_json::to_string(&store.expire_past(paydirt::parse_id(tenant).ok_or("invalid tenant UUID")?,&req.window,req.dry_run)?)?);
        return Ok(());
    }
    if let [command, entity, id]=args.as_slice() && command=="trail-row" {
        let mut store=__APP_CRATE__::resource::record(paydirt_store::Store::open(&url,MODEL,POLICIES)?)?;
        let f=store.verify_object(entity,paydirt::parse_id(id).ok_or("invalid row UUID")?)?;
        println!("{}",serde_json::to_string(&f)?);
        return Ok(());
    }
    if let [command, argument] = args.as_slice() && matches!(command.as_str(),"trail"|"trail-check") {
        use paydirt_store::archive_files::Archive;
        let archive=Archive::from_environment;
        let vault=archive("PAYDIRT_ARCHIVE_DIR","PAYDIRT_ARCHIVE_KEY")?;
        let export=std::env::var_os("PAYDIRT_EXPORT_DIR").map(|_|archive("PAYDIRT_EXPORT_DIR","PAYDIRT_EXPORT_KEY")).transpose()?;
        let mut store=__APP_CRATE__::resource::record(paydirt_store::Store::open(&url,MODEL,POLICIES)?)?;
        if command=="trail-check"{
            let advance=match argument.as_str(){"checkpoint"=>true,"verify"=>false,_=>return Err("expected verify or checkpoint".into())};
            println!("{}",serde_json::to_string(&store.attest_trail(&vault,advance)?)?);
            return Ok(());
        }
        println!("{}",serde_json::to_string(&store.manage_trail(&vault,export.as_ref(),serde_json::from_str(argument)?)?)?);
        return Ok(())
    }
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
            __APP_CRATE__::resource::record(paydirt_store::Store::open(&url, MODEL, POLICIES)?)?
                .with_outbox(config.watch.outbox);
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
    let store =
        __APP_CRATE__::resource::record(paydirt_store::Store::open(&url, MODEL, POLICIES)?)?
            .with_outbox(config.watch.outbox);
    let mut db = Database::from_store(store, &principals)?
        .with_broker(broker.clone())
        .allow_general_writes(config.allow_general_writes);
    if let Some(rate) = config.rate {
        db = db.with_rate(paydirt_server::Limiter::new(rate).map_err(|e| e as Box<dyn Error>)?);
    }
    if let Some(auth) = config.authentication {
        db = db.with_authentication(auth)?;
    }
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
    use paydirt::spin::{Lease, Placement};
    let mut workers = __APP_CRATE__::workers(&db)?;
    if let Some(ms) = config.notification_expiry_ms {
        if ms == 0 {
            return Err("notification expiry interval must be positive".into());
        }
        let url = url.clone();
        let broker = broker.clone();
        let outbox = config.watch.outbox;
        workers.add("payday.notify.expire",Placement::Singleton,move |stop|{
            let url=url.clone();
            let broker=broker.clone();
            async move {
                let mut store=paydirt_store::Store::open(&url,MODEL,POLICIES).map_err(|e|e.to_string())?.with_outbox(outbox);
                store.notification_keep().map_err(|e|e.to_string())?;
                loop {
                    if stop.stopped(){return Ok(())}
                    match store.expire_notifications(None,500) {
                        Ok(n)=>{
                            if let (Some(event),Some(b))=(store.take_committed(),broker.as_ref()){
                                if let Err(e)=b.publish(event).await{eprintln!("notification expiry invalidation: {e}")}
                            }
                            if n==500{tokio::task::yield_now().await;continue}
                        }
                        Err(e)=>eprintln!("notification expiry: {e}"),
                    }
                    tokio::select!{_=stop.cancelled()=>return Ok(()),_=tokio::time::sleep(Duration::from_millis(ms))=>{}}
                }
            }
        })?;
    }
    if let (Some(ms), Some(broker)) = (config.watch.drain_every_ms, broker) {
        let url = url.clone();
        workers.add("paydirt.outbox", Placement::Singleton, move |stop| {
            let url = url.clone();
            let broker = broker.clone();
            async move {
                let mut store =
                    paydirt_store::Store::open(&url, MODEL, POLICIES).map_err(|e| e.to_string())?;
                let mut tick = tokio::time::interval(Duration::from_millis(ms));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {_=stop.cancelled()=>return Ok(()),_=tick.tick()=>{}}
                    if let Err(err) = store.drain_once(&*broker, 128).await {
                        eprintln!("outbox drain failed: {err}");
                    }
                }
            }
        })?;
    }
    if let Some(meter) = config.meter {
        let every = Duration::from_millis(meter.every_ms.unwrap_or(86_400_000));
        let retry = Duration::from_millis(meter.retry_ms.unwrap_or(60_000));
        if every.is_zero() || retry.is_zero() || meter.directory.as_os_str().is_empty() {
            return Err("meter intervals and directory are required".into());
        }
        let url = url.clone();
        workers.add("payday.meter.read", Placement::Singleton, move |stop| {
            let url = url.clone();
            let dir = meter.directory.clone();
            async move {
                let mut store =
                    paydirt_store::Store::open(&url, MODEL, POLICIES).map_err(|e| e.to_string())?;
                paydirt_store::meter::periodic(stop, every, retry, |pass| {
                    pass.run(&mut store, |rows| {
                        paydirt_store::meter::directory_sink(&dir, rows)
                    })
                })
                .await
                .map_err(|e| e.to_string())
            }
        })?;
    }
    if let Some(source) = config.grant_source {
        grants::register(&mut workers, &url, source)?;
    }
    let lease = match config.worker_lease.as_deref() {
        None | Some("local") => Lease::Local,
        Some("postgres") => Lease::Postgres {
            dsn: url,
            retry: Duration::from_secs(1),
        },
        Some(v) => return Err(format!("unknown worker lease {v}").into()),
    };
    let routes = router(db, config.service_name.clone());
    let listener = paydirt_server::Host::bind(addr, config.serving, &config.service_name, routes)
        .await
        .map_err(|e| e as Box<dyn Error>)?;
    let stop = async {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            if let Ok(mut term) = signal(SignalKind::terminate()) {
                tokio::select! {_=term.recv()=>{},_=tokio::signal::ctrl_c()=>{}}
                return;
            }
        }
        let _ = tokio::signal::ctrl_c().await;
    };
    workers
        .run(lease, stop, move |stop| async move {
            listener.serve(stop).await.map_err(|e| e.to_string())
        })
        .await
        .map_err(Into::into)
}
