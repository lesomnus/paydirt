//! Application-owned HTTP routes on the primary and optional browser listeners.
pub fn http(_db: &__APP_CRATE__::transport::Database) -> paydirt_server::axum::Router {
    paydirt_server::axum::Router::new()
}
