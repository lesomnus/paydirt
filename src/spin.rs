//! Cooperative server background workers and PostgreSQL session leases.
use futures_util::future::BoxFuture;
use std::{collections::BTreeMap, future::Future, sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinSet};

pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone)]
pub struct Stop(watch::Receiver<bool>);
impl Stop {
    pub fn stopped(&self) -> bool {
        *self.0.borrow()
    }
    pub async fn cancelled(&self) {
        let mut rx = self.0.clone();
        let _ = rx.wait_for(|v| *v).await;
    }
}

#[derive(Clone)]
pub enum Lease {
    Local,
    Postgres { dsn: String, retry: Duration },
}
#[derive(Copy, Clone)]
pub enum Placement {
    Everywhere,
    Singleton,
}
type Work = Arc<dyn Fn(Stop) -> BoxFuture<'static, Result<()>> + Send + Sync>;

#[derive(Default)]
pub struct Workers(BTreeMap<String, (Placement, Work)>);
impl Workers {
    pub fn add<F, Fut>(
        &mut self,
        name: impl Into<String>,
        placement: Placement,
        work: F,
    ) -> Result<()>
    where
        F: Fn(Stop) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let name = name.into();
        if name.trim().is_empty() || self.0.contains_key(&name) {
            return Err(format!("empty or duplicate worker name {name:?}"));
        }
        self.0
            .insert(name, (placement, Arc::new(move |s| Box::pin(work(s)))));
        Ok(())
    }
    /// Bind the listener before calling run. A fatal worker or server error stops siblings.
    pub async fn run<S, SF, StopF>(self, lease: Lease, shutdown: StopF, serve: S) -> Result<()>
    where
        S: FnOnce(Stop) -> SF + Send + 'static,
        SF: Future<Output = Result<()>> + Send + 'static,
        StopF: Future<Output = ()>,
    {
        let (stop, rx) = watch::channel(false);
        let mut tasks = JoinSet::new();
        let mut keys = BTreeMap::new();
        if let Lease::Postgres { retry, .. } = &lease {
            if retry.is_zero() {
                return Err("lease retry must be positive".into());
            }
            for (name, (p, _)) in &self.0 {
                if matches!(p, Placement::Singleton)
                    && let Some(other) = keys.insert(key(name), name)
                {
                    return Err(format!("lease key collision between {name} and {other}"));
                }
            }
        }
        for (name, (placement, work)) in self.0 {
            let token = Stop(rx.clone());
            let lease = lease.clone();
            tasks.spawn(async move {
                let res = match (placement, lease) {
                    (Placement::Singleton, Lease::Postgres { dsn, retry }) => {
                        leased(&name, &dsn, retry, token, work).await
                    }
                    _ => work(token).await,
                };
                res.map(|_| false).map_err(|e| format!("{name}: {e}"))
            });
        }
        tasks.spawn(async move { serve(Stop(rx)).await.map(|_| true) });
        tokio::pin!(shutdown);
        let mut first = None;
        loop {
            tokio::select! {
                _=&mut shutdown=>break,
                task=tasks.join_next()=>match task{
                    Some(Ok(Ok(false)))=>continue,
                    Some(Ok(Ok(true)))|None=>break,
                    Some(Ok(Err(e)))=>{first=Some(e);break;},
                    Some(Err(e))=>{first=Some(e.to_string());break;},
                }
            }
        }
        stop.send_replace(true);
        while let Some(done) = tasks.join_next().await {
            match done {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    first.get_or_insert(e);
                }
                Err(e) => {
                    first.get_or_insert(e.to_string());
                }
            }
        }
        first.map_or(Ok(()), Err)
    }
}
const NAMESPACE: i32 = 0x70647370;
fn key(name: &str) -> i32 {
    name.bytes().fold(0x811c9dc5u32, |v, b| {
        (v ^ u32::from(b)).wrapping_mul(16777619)
    }) as i32
}

async fn leased(name: &str, dsn: &str, retry: Duration, stop: Stop, work: Work) -> Result<()> {
    let key = key(name);
    loop {
        if stop.stopped() {
            return Ok(());
        }
        let connection = tokio::select! {_=stop.cancelled()=>return Ok(()),c=tokio_postgres::connect(dsn,tokio_postgres::NoTls)=>c};
        if let Ok((client, driver)) = connection {
            let mut driver = tokio::spawn(driver);
            let params: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[&NAMESPACE, &key];
            let lock = tokio::select! {_=stop.cancelled()=>None,rs=client.query_one("SELECT pg_try_advisory_lock($1,$2)",params)=>rs.ok().map(|r|r.get::<_,bool>(0))};
            if lock == Some(true) {
                let (end, rx) = watch::channel(false);
                let job = work(Stop(rx));
                tokio::pin!(job);
                let ping = async {
                    loop {
                        tokio::time::sleep(retry.max(Duration::from_millis(100))).await;
                        if !matches!(
                            tokio::time::timeout(
                                retry.max(Duration::from_secs(2)),
                                client.simple_query("SELECT 1")
                            )
                            .await,
                            Ok(Ok(_))
                        ) {
                            break;
                        }
                    }
                };
                let complete = tokio::select! {r=&mut job=>Some(r),_=stop.cancelled()=>None,_=&mut driver=>None,_=ping=>None};
                end.send_replace(true);
                if let Some(result) = complete {
                    drop(client);
                    driver.abort();
                    return result;
                }
                job.await?;
            }
            // Closing this dedicated session releases its advisory lock.
            drop(client);
            driver.abort();
        }
        tokio::select! {_=stop.cancelled()=>return Ok(()),_=tokio::time::sleep(retry)=>{}}
    }
}
