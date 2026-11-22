mod core;
#[path = "../generated/messages/mod.rs"]
pub mod proto;
#[path = "../generated/services/mod.rs"]
pub mod services;
mod sink;
pub mod transport;
pub mod storage {
    include!("../generated/storage/sinks.rs");
}

pub fn router(database: transport::Database, name: String) -> connectrpc::Router {
    struct App(String);
    impl storage::Layers for App {
        fn public_methods(&self) -> &'static [&'static str] {
            &["/app.SettingService/Get"]
        }
        fn app_setting(
            &self,
            next: impl services::app::SettingService,
        ) -> impl services::app::SettingService {
            let settings = services::app::SettingServiceOverlay {
                next,
                layer: sink::Settings {
                    name: self.0.clone(),
                },
            };
            services::app::SettingServiceOverlay {
                next: settings,
                layer: core::Core,
            }
        }
    }
    storage::mount(connectrpc::Router::new(), database, &App(name))
}

#[cfg(target_arch = "wasm32")]
mod worker;

/// Register application background work before starting the native listener.
#[cfg(not(target_arch = "wasm32"))]
pub fn workers(_database: &transport::Database) -> Result<paydirt::spin::Workers, String> {
    Ok(paydirt::spin::Workers::default())
}
