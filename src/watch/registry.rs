//! Application-owned factories are registered before the server starts.
use super::{Broker, Result};
use futures_util::future::BoxFuture;
use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock, RwLock},
};

#[derive(Clone)]
pub struct BrokerSettings {
    pub database: String,
    pub dsn: String,
    pub namespace: String,
}
type Factory =
    dyn Fn(BrokerSettings) -> BoxFuture<'static, Result<Option<Arc<dyn Broker>>>> + Send + Sync;
static FACTORIES: LazyLock<RwLock<BTreeMap<String, Arc<Factory>>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));
const BUILTINS: [&str; 4] = ["memory", "nats", "none", "postgres"];

pub fn register_broker(
    name: &str,
    factory: impl Fn(BrokerSettings) -> BoxFuture<'static, Result<Option<Arc<dyn Broker>>>>
    + Send
    + Sync
    + 'static,
) -> Result<()> {
    if name.is_empty() || BUILTINS.contains(&name) {
        return Err(format!("reserved or empty broker name {name:?}"));
    }
    let mut factories = FACTORIES
        .write()
        .map_err(|_| "watch broker registry poisoned")?;
    if factories.contains_key(name) {
        return Err(format!("watch broker {name} is already registered"));
    }
    factories.insert(name.into(), Arc::new(factory));
    Ok(())
}
pub fn broker_names() -> Vec<String> {
    let mut names = BUILTINS.map(str::to_owned).to_vec();
    if let Ok(map) = FACTORIES.read() {
        names.extend(map.keys().cloned());
    }
    names.sort();
    names
}
pub async fn registered_broker(name: &str, settings: BrokerSettings) -> Result<Arc<dyn Broker>> {
    let factory = FACTORIES
        .read()
        .map_err(|_| "watch broker registry poisoned")?
        .get(name)
        .cloned();
    let factory = factory.ok_or_else(|| {
        format!(
            "unknown watch.broker {name:?}. Choose from {} or register it with register_broker",
            broker_names().join(", ")
        )
    })?;
    factory(settings)
        .await?
        .ok_or_else(|| format!("watch broker {name:?} built no broker. Use none to disable Watch"))
}
