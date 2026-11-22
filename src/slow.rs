//! Timing for native unary calls and the reads they perform below the gate.
use std::{
    borrow::Cow,
    io::{self, Write},
    sync::LazyLock,
    time::{Duration, Instant},
};

static THRESHOLD: LazyLock<Duration> = LazyLock::new(|| {
    let millis = std::env::var(crate::environment::PAYDIRT_SLOW_MS)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(250);
    Duration::from_millis(millis)
});

pub struct Timer(Instant);
impl Timer {
    pub fn start() -> Self {
        Self(Instant::now())
    }

    /// Delay constructing the name until there is a diagnostic to emit.
    pub fn finish<'a>(self, method: impl FnOnce() -> Cow<'a, str>) {
        let elapsed = self.0.elapsed();
        if elapsed >= *THRESHOLD {
            let record = serde_json::json!({
                "level": "WARN", "msg": "slow", "rpc.method": method(),
                "elapsed": elapsed.as_nanos().min(u64::MAX as u128) as u64
            });
            let _ = writeln!(io::stderr().lock(), "{record}");
        }
    }
}
