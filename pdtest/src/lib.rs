//! Reusable fixtures for testing an application's normal RPC router in-process.
use connectrpc::{
    ConnectError, ConnectRpcService, ErrorCode, Router,
    client::{ClientConfig, ServiceTransport},
};
use paydirt_store::{Inputs, Store};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub fn clock() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(981173106, 0).unwrap()
}
/// Stable UUIDv8 test identifier. The counter is local to the fixture.
pub fn id(domain: u8, counter: u64) -> [u8; 16] {
    let mut bytes = [0; 16];
    bytes[..6].copy_from_slice(&clock().timestamp_millis().to_be_bytes()[2..]);
    let n = counter.to_be_bytes();
    bytes[6] = 0x80 | (n[0] & 15);
    bytes[7] = n[1];
    bytes[8] = 0x80 | (n[0] >> 4);
    bytes[9] = domain;
    bytes[10..].copy_from_slice(&n[2..]);
    bytes
}
/// Each fixture owns a temporary SQLite database and an independent ID sequence.
/// Reopening its store preserves both rows and the next identifier.
pub struct Fixture {
    dir: tempfile::TempDir,
    sequence: Arc<AtomicU64>,
}
impl Fixture {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            dir: tempfile::tempdir()?,
            sequence: Default::default(),
        })
    }
    pub fn store(&self, model: &str, policies: &str) -> paydirt_store::Result<Store> {
        let path = self.dir.path().join("app.sqlite3");
        let sequence = self.sequence.clone();
        Ok(Store::open(
            path.to_str()
                .ok_or_else(|| paydirt_store::Fault::invalid("database path is not UTF-8"))?,
            model,
            policies,
        )?
        .with_inputs(Inputs::new(clock, move |domain| {
            id(domain, sequence.fetch_add(1, Ordering::Relaxed) + 1)
        })))
    }
}
pub fn client(router: Router) -> (ServiceTransport<ConnectRpcService>, ClientConfig) {
    (
        ServiceTransport::new(ConnectRpcService::new(router)),
        ClientConfig::new("http://app.test".parse().unwrap()),
    )
}
#[track_caller]
pub fn assert_code<T>(result: Result<T, ConnectError>, expected: ErrorCode) {
    match result {
        Err(error) => assert_eq!(error.code, expected, "{error}"),
        Ok(_) => panic!("expected {expected:?}, call succeeded"),
    }
}
