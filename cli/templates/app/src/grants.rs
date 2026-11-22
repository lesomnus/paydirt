use crate::operator::rpc_client;
use __APP_CRATE__::{
    proto::payday::{GrantListRequest, GrantListResponse, GrantWatchRequest, GrantWatchResponse},
    transport::{MODEL, POLICIES},
};
use buffa::HasMessageView;
use connectrpc::client::{ClientConfig, HttpClient, call_server_stream, call_unary};
use paydirt::spin::{Placement, Stop, Workers};
use paydirt_store::Store;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeSet, time::Duration};
use tokio::time::{Instant, sleep};

#[derive(Deserialize)]
pub struct Source {
    url: String,
    pub(crate) bearer_env: Option<String>,
    every_ms: Option<u64>,
    retry_ms: Option<u64>,
}
pub fn register(workers: &mut Workers, url: &str, source: Source) -> Result<(), String> {
    let policies: paydirt_store::Policies =
        serde_json::from_str(POLICIES).map_err(|e| e.to_string())?;
    if !policies
        .values()
        .any(|v| v.entity["own"] == "OWN_ENTITLEMENT")
    {
        return Err("grant_source requires a schema with entitlements".into());
    }
    let token = source
        .bearer_env
        .map(std::env::var)
        .transpose()
        .map_err(|e| e.to_string())?;
    let (http, config) = rpc_client(&source.url, token.as_deref()).map_err(|e| e.to_string())?;
    let every = Duration::from_millis(source.every_ms.unwrap_or(3_600_000));
    let retry = Duration::from_millis(source.retry_ms.unwrap_or(1_000));
    if every.is_zero() || retry.is_zero() {
        return Err("grant source intervals must be positive".into());
    }
    let url = url.to_owned();
    workers.add("payday.entitle.follow",Placement::Singleton,move |stop|{
        let url=url.clone();let http=http.clone();let config=config.clone();
        async move {
            let mut store=Store::open(&url,MODEL,POLICIES).map_err(|e|e.to_string())?;
            let mut delay=retry;
            loop {
                let began=Instant::now();let mut ready=false;
                tokio::select!{
                    _=stop.cancelled()=>return Ok(()),
                    res=follow(&mut store,&http,&config,every,retry,&mut ready)=>if let Err(e)=res{eprintln!("grant watch: {e}");}
                }
                if ready && began.elapsed()>=Duration::from_secs(60) { delay=retry; }
                if wait(&stop,delay).await{return Ok(());}
                delay=delay.saturating_mul(2).min(Duration::from_secs(60));
            }
        }
    })
}
async fn wait(stop: &Stop, delay: Duration) -> bool {
    tokio::select! {_=stop.cancelled()=>true,_=sleep(delay)=>false}
}
type Error = Box<dyn std::error::Error + Send + Sync>;
async fn snapshot(http: &HttpClient, config: &ClientConfig) -> Result<Vec<Value>, Error> {
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    let mut cursor = String::new();
    loop {
        let page = call_unary::<
            _,
            GrantListRequest,
            <GrantListResponse as HasMessageView>::View<'static>,
        >(
            http,
            config,
            "payday.GrantService",
            "List",
            GrantListRequest {
                size: 500,
                cursor,
                ..Default::default()
            },
            Default::default(),
        )
        .await?
        .into_owned();
        for grant in page.items {
            rows.push(serde_json::to_value(grant)?);
        }
        cursor = page.next;
        if cursor.is_empty() {
            return Ok(rows);
        }
        if !seen.insert(cursor.clone()) {
            return Err("grant list cursor repeated".into());
        }
    }
}
async fn follow(
    store: &mut Store,
    http: &HttpClient,
    config: &ClientConfig,
    every: Duration,
    retry: Duration,
    ready: &mut bool,
) -> Result<(), Error> {
    let mut stream = call_server_stream::<
        _,
        GrantWatchRequest,
        <GrantWatchResponse as HasMessageView>::View<'static>,
    >(
        http,
        config,
        "payday.GrantService",
        "Watch",
        Default::default(),
        Default::default(),
    )
    .await?;
    let first = stream
        .message::<GrantWatchResponse>()
        .await?
        .ok_or("grant stream ended before ready")?;
    if serde_json::to_value(first.to_owned_message())?
        .get("ready")
        .is_none()
    {
        return Err("grant stream did not begin with ready".into());
    }
    *ready = true;
    let mut delay = Duration::ZERO;
    let mut backoff = retry;
    loop {
        let wait_for = delay;
        let reconcile = async {
            sleep(wait_for).await;
            snapshot(http, config).await
        };
        tokio::pin!(reconcile);
        loop {
            tokio::select! {
                result=&mut reconcile=>{
                    match result.and_then(|rows|Ok(store.apply_grants(None,&rows,true)?)){
                        Ok(_)=>{delay=every;backoff=retry;}
                        Err(e)=>{eprintln!("grant reconciliation: {e}");delay=backoff;backoff=backoff.saturating_mul(2).min(Duration::from_secs(60));}
                    }
                    break;
                }
                msg=stream.message::<GrantWatchResponse>()=>{
                    let event=serde_json::to_value(msg?.ok_or("grant stream ended")?.to_owned_message())?;
                    if let Some(row)=event.get("grant"){ store.apply_grants(None,std::slice::from_ref(row),false)?; }
                }
            }
        }
    }
}
