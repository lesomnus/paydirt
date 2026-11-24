use paydirt_server::{Host, Serving};

#[tokio::test]
async fn ephemeral_primary_address_identifies_the_bound_listener() {
    let host = Host::bind(
        "127.0.0.1:0".parse().unwrap(),
        Serving::default(),
        "address-test",
        connectrpc::Router::new(),
    )
    .await
    .unwrap();
    let address = host.local_addr().unwrap();
    assert!(address.ip().is_loopback());
    assert_ne!(address.port(), 0);
    assert!(tokio::net::TcpListener::bind(address).await.is_err());
    drop(host);
    assert!(tokio::net::TcpListener::bind(address).await.is_ok());
}
