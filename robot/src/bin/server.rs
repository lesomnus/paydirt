#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = paydirt_robot::Backend::open(&std::env::var("DATABASE_URL")?)?;
    let router = paydirt_robot::router(db, &std::env::var("PAYDIRT_PRINCIPALS")?)?;
    let address = std::env::var("PAYDIRT_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".into());
    connectrpc::Server::new(router)
        .serve(address.parse()?)
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)
}
