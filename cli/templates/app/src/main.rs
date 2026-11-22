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

impl Config {
    fn load() -> Result<Self, Box<dyn Error>> {
        let mut config: Self = serde_json::from_slice(&fs::read("app.json")?)?;
        if let Some(broker) = std::env::var_os(paydirt::environment::PAYDIRT_WATCH_BROKER) {
            config.watch.broker = broker.into_string().map_err(|_| "invalid watch broker")?;
        }
        if let Some(lease) = std::env::var_os(paydirt::environment::PAYDIRT_WORKER_LEASE) {
            config.worker_lease = Some(lease.into_string().map_err(|_| "invalid worker lease")?);
        }
        Ok(config)
    }

    fn print_environment(&self, state: bool) -> Result<(), Box<dyn Error>> {
        let mut names: std::collections::BTreeSet<_> =
            paydirt::environment::NAMES.iter().copied().collect();
        names.extend(self.authentication.as_ref().and_then(|v| v.introspection.as_ref()).map(|v| v.bearer_env.as_str()));
        names.extend(self.grant_source.as_ref().and_then(|v| v.bearer_env.as_deref()));
        for name in &names {
            if name.is_empty() || !name.bytes().enumerate().all(|(i,c)| c.is_ascii_alphabetic() || c == b'_' || (i > 0 && c.is_ascii_digit())) {
                return Err("invalid environment variable name in app.json".into());
            }
        }
        for name in names {
            if state {
                let mark = if std::env::var_os(name).is_some() { "set" } else { "-" };
                println!("{name}\t{mark}");
            } else {
                println!("{name}");
            }
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = std::env::var(paydirt::environment::DATABASE_URL).unwrap_or_else(|_| "paydirt.sqlite3".into());
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("config") {
        return match args.as_slice() {
            [_, cmd] if cmd == "env" => Config::load()?.print_environment(false),
            [_, cmd, flag] if cmd == "env" && flag == "--set" => Config::load()?.print_environment(true),
            [_] | [_, _] if args.len() == 1 || matches!(args[1].as_str(), "-h" | "--help") => { println!("env"); Ok(()) },
            [_, cmd, flag] if cmd == "env" && matches!(flag.as_str(), "-h" | "--help") => { println!("config env [--set]"); Ok(()) },
            _ => Err("usage: config env [--set]".into()),
        };
    }
    if let [command, tenant, config]=args.as_slice() && command=="past-expire" {
        #[derive(Deserialize)]
        struct Expiry {window:paydirt_store::past_expiry::Window,#[serde(default)]dry_run:bool}
        let req:Expiry=serde_json::from_str(config)?;
        let mut store=__APP_CRATE__::resource::record(paydirt_store::Store::open(&url,MODEL,POLICIES)?)?;
        println!("{}",serde_json::to_string(&store.expire_past(paydirt::parse_id(tenant).ok_or("invalid tenant UUID")?,&req.window,req.dry_run)?)?);
        return Ok(());
    }
    if let [command, entity, request]=args.as_slice() && command=="trail-record" {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Record { action: String, ids: Vec<String>, #[serde(default)] erased:bool }
        let req:Record=serde_json::from_str(request)?;
        let ids=req.ids.iter().map(|v|paydirt::parse_id(v).ok_or("invalid row UUID")).collect::<Result<Vec<_>,_>>()?;
        let mut store=__APP_CRATE__::resource::record(paydirt_store::Store::open(&url,MODEL,POLICIES)?)?;
        let archive=paydirt_store::archive_files::Archive::verification_from_environment()?;
        println!("{}",store.transaction(&paydirt_store::Scope::Internal,|tx|tx.record_objects_with_archive(entity,&ids,&req.action,req.erased,archive.as_ref()))?);
        return Ok(());
    }
    if let [command, entity]=args.as_slice() && command=="trail-scan" {
        let archive=paydirt_store::archive_files::Archive::verification_from_environment()?;
        let mut store=__APP_CRATE__::resource::record(paydirt_store::Store::open(&url,MODEL,POLICIES)?)?;
        let report=store.check_kind(entity,archive.as_ref())?;
        println!("{}",serde_json::to_string(&report)?);
        return Ok(());
    }
    if let [command, entity, id]=args.as_slice() && command=="trail-row" {
        let mut store=__APP_CRATE__::resource::record(paydirt_store::Store::open(&url,MODEL,POLICIES)?)?;
        let archive=paydirt_store::archive_files::Archive::verification_from_environment()?;
        let f=store.verify_object_with_archive(entity,paydirt::parse_id(id).ok_or("invalid row UUID")?,archive.as_ref())?;
        println!("{}",serde_json::to_string(&f)?);
        return Ok(());
    }
    if matches!(args.first().map(String::as_str),Some("trail-pass"|"trail-receipts")) {
        let vault=paydirt_store::archive_files::Archive::from_environment(paydirt::environment::PAYDIRT_ARCHIVE_DIR,paydirt::environment::PAYDIRT_ARCHIVE_KEY)?;
        let result=match args.as_slice(){
            [cmd] if cmd=="trail-receipts"=>serde_json::to_value(vault.trail_receipts()?)?,
            _ if args[0]=="trail-pass" && (args.len()==1 || args[1..]==["--dry-run"])=>{
                let mut store=__APP_CRATE__::resource::record(paydirt_store::Store::open(&url,MODEL,POLICIES)?)?;
                serde_json::to_value(store.sweep_trail(&vault,args.len()==2)?)?
            }
            _=>return Err("expected trail-pass [--dry-run] or trail-receipts".into()),
        };
        println!("{}",serde_json::to_string(&result)?);
        return Ok(());
    }
    if let [command, argument] = args.as_slice() && matches!(command.as_str(),"trail"|"trail-check"|"trail-read") {
        use paydirt_store::archive_files::Archive;
        let archive=Archive::from_environment;
        let vault=archive(paydirt::environment::PAYDIRT_ARCHIVE_DIR,paydirt::environment::PAYDIRT_ARCHIVE_KEY)?;
        let export=std::env::var_os(paydirt::environment::PAYDIRT_EXPORT_DIR).map(|_|archive(paydirt::environment::PAYDIRT_EXPORT_DIR,paydirt::environment::PAYDIRT_EXPORT_KEY)).transpose()?;
        let mut store=__APP_CRATE__::resource::record(paydirt_store::Store::open(&url,MODEL,POLICIES)?)?;
        if command=="trail-read"{
            let tenant=paydirt::parse_id(argument).ok_or("invalid tenant identifier")?;
            println!("{}",serde_json::to_string(&store.read_tenant_trail(&vault,tenant)?)?);
            return Ok(());
        }
        if command=="trail-check"{
            let result=match argument.as_str(){
                "checkpoint"|"verify"=>serde_json::to_value(store.attest_trail(&vault,argument=="checkpoint")?)?,
                "review"=>serde_json::to_value(store.review_trail(&vault)?)?,
                json if json.trim_start().starts_with('{')=>serde_json::to_value(store.accept_trail_restore(&vault,serde_json::from_str(json)?)?)?,
                _=>return Err("expected verify, checkpoint, review or a restore acceptance".into()),
            };
            println!("{}",serde_json::to_string(&result)?);
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
        let config = Config::load()?;
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
    let config = Config::load()?;
    let addr = match std::env::var(paydirt::environment::PAYDIRT_LISTEN) {
        Ok(addr) => addr.parse()?,
        Err(std::env::VarError::NotPresent) => config.listen,
        Err(e) => return Err(e.into()),
    };
    let principals = std::env::var(paydirt::environment::PAYDIRT_PRINCIPALS).unwrap_or_else(|_| "{}".into());
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
    if let Ok(path) = std::env::var(paydirt::environment::PAYDIRT_POLICY_FILE) {
        let refresh = std::env::var(paydirt::environment::PAYDIRT_POLICY_REFRESH_MS)
            .ok()
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(5000);
        db = db.with_policy(paydirt::rbac::Policy::file(
            path.into(),
            std::time::Duration::from_millis(refresh),
        )?);
    }
    if let Ok(header) = std::env::var(paydirt::environment::PAYDIRT_ACCOUNT_HEADER) {
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
