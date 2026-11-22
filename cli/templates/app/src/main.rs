mod core;
#[path = "../generated/messages/mod.rs"]
mod proto;
#[path = "../generated/services/mod.rs"]
mod services;
mod sink;
mod orm {
    include!("../generated/orm/app/setting_svc.g.orm.rs");
}

use connectrpc::{Router, Server};
use serde::Deserialize;
use std::{error::Error, fs, sync::Arc};

#[derive(Deserialize)]
struct Config {
    listen: std::net::SocketAddr,
    service_name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config: Config = serde_json::from_slice(&fs::read("app.json")?)?;
    let address = match std::env::var("PAYDIRT_LISTEN") {
        Ok(value) => value.parse()?,
        Err(std::env::VarError::NotPresent) => config.listen,
        Err(err) => return Err(err.into()),
    };
    use services::app::{SettingServiceOverlay, SettingServiceUnimplemented};
    let settings = SettingServiceOverlay {
        next: SettingServiceUnimplemented,
        layer: sink::Settings {
            name: config.service_name,
        },
    };
    let service = Arc::new(SettingServiceOverlay {
        next: settings,
        layer: core::Core,
    });
    let router = orm::register_setting_service(Router::new(), service);
    Server::new(router)
        .serve(address)
        .await
        .map_err(|e| e as Box<dyn Error>)
}
