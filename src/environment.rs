//! Environment overrides supported by generated native applications.
macro_rules! keys {
    ($($name:ident),+ $(,)?) => {
        $(pub const $name: &str = stringify!($name);)+
        pub const NAMES: &[&str] = &[$($name),+];
    };
}
keys! {
    APP_TZ,
    DATABASE_URL,
    PAYDIRT_ACCOUNT_HEADER,
    PAYDIRT_ARCHIVE_DIR,
    PAYDIRT_ARCHIVE_KEY,
    PAYDIRT_AUDIT_KEY,
    PAYDIRT_EXPORT_DIR,
    PAYDIRT_EXPORT_KEY,
    PAYDIRT_LISTEN,
    PAYDIRT_POLICY_FILE,
    PAYDIRT_POLICY_REFRESH_MS,
    PAYDIRT_PRINCIPALS,
    PAYDIRT_SIGNER_ARGS,
    PAYDIRT_SIGNER_PROGRAM,
    PAYDIRT_SIGNER_PUBLIC_KEY,
    PAYDIRT_SIGNING_KEY,
    PAYDIRT_SLOW_MS,
    PAYDIRT_TOKEN,
    PAYDIRT_TRUST_KEYS,
    PAYDIRT_URL,
    PAYDIRT_WATCH_BROKER,
    PAYDIRT_WITNESS_DIR,
    PAYDIRT_WORKER_LEASE,
}
