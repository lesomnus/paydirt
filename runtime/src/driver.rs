use crate::{Fault, Result};
use diesel::Connection as _;
use diesel::{
    connection::SimpleConnection,
    prelude::*,
    result::{DatabaseErrorKind, Error},
    sql_types::*,
};
use paydirt_model::Dialect;

#[derive(Debug, Clone)]
pub(crate) enum Bind {
    Text(String),
    Bytes(Vec<u8>),
    Integer(i32),
    Big(i64),
    Float(f64),
    Bool(bool),
}

#[derive(Default)]
pub(crate) struct Query {
    pub sql: String,
    pub args: Vec<Bind>,
}
impl Query {
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            args: Vec::new(),
        }
    }
    pub fn param(&mut self, value: Bind, d: Dialect) -> String {
        self.args.push(value);
        match d {
            Dialect::Postgres => format!("${}", self.args.len()),
            Dialect::Sqlite => format!("?{}", self.args.len()),
        }
    }
}
#[derive(QueryableByName)]
struct Row {
    #[diesel(sql_type=Text)]
    value: String,
}

impl From<Error> for Fault {
    fn from(e: Error) -> Self {
        match e {
            Error::NotFound => Self::missing(),
            Error::DatabaseError(DatabaseErrorKind::UniqueViolation, _) => {
                Self::conflict("key already exists")
            }
            Error::DatabaseError(DatabaseErrorKind::ForeignKeyViolation, _) => {
                Self::precondition("referenced row missing or in use")
            }
            Error::DatabaseError(DatabaseErrorKind::SerializationFailure, _) => {
                Self::aborted("write raced another transaction")
            }
            e => Self::internal(e.to_string()),
        }
    }
}
pub(crate) trait Driver {
    fn dialect(&self) -> Dialect;
    fn exec(&mut self, query: Query) -> Result<usize>;
    fn rows(&mut self, query: Query) -> Result<Vec<String>>;
    fn batch(&mut self, sql: &str) -> Result<()>;
}
#[allow(unused_macros)]
macro_rules! bound {
    ($query:expr,$backend:ty) => {{
        let mut sql = diesel::sql_query($query.sql).into_boxed::<$backend>();
        for arg in $query.args {
            sql = match arg {
                Bind::Text(v) => sql.bind::<Text, _>(v),
                Bind::Bytes(v) => sql.bind::<Binary, _>(v),
                Bind::Integer(v) => sql.bind::<Integer, _>(v),
                Bind::Big(v) => sql.bind::<BigInt, _>(v),
                Bind::Float(v) => sql.bind::<Double, _>(v),
                Bind::Bool(v) => sql.bind::<Bool, _>(v),
            };
        }
        sql
    }};
}
#[allow(unused_macros)]
macro_rules! driver {
    ($conn:ty,$backend:ty,$dialect:expr) => {
        impl Driver for $conn {
            fn dialect(&self) -> Dialect {
                $dialect
            }
            fn exec(&mut self, q: Query) -> Result<usize> {
                Ok(bound!(q, $backend).execute(self)?)
            }
            fn rows(&mut self, q: Query) -> Result<Vec<String>> {
                Ok(bound!(q, $backend)
                    .load::<Row>(self)?
                    .into_iter()
                    .map(|r| r.value)
                    .collect())
            }
            fn batch(&mut self, sql: &str) -> Result<()> {
                Ok(self.batch_execute(sql)?)
            }
        }
    };
}
#[cfg(feature = "postgres")]
driver!(PgConnection, diesel::pg::Pg, Dialect::Postgres);
#[cfg(feature = "sqlite")]
driver!(SqliteConnection, diesel::sqlite::Sqlite, Dialect::Sqlite);

pub enum Connection {
    #[cfg(feature = "postgres")]
    Postgres(PgConnection),
    #[cfg(feature = "sqlite")]
    Sqlite(SqliteConnection),
}
impl Connection {
    pub fn open(url: &str) -> Result<Self> {
        #[cfg(feature = "postgres")]
        if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            return PgConnection::establish(url)
                .map(Self::Postgres)
                .map_err(|e| Fault::internal(e.to_string()));
        }
        #[cfg(feature = "sqlite")]
        if !url.contains("://") || url.starts_with("sqlite://") {
            let mut db = SqliteConnection::establish(url.strip_prefix("sqlite://").unwrap_or(url))
                .map_err(|e| Fault::internal(e.to_string()))?;
            db.batch_execute("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;
            return Ok(Self::Sqlite(db));
        }
        Err(Fault::invalid(format!(
            "no compiled database driver for scheme {}",
            url.split(':').next().unwrap_or("")
        )))
    }
    pub(crate) fn driver(&mut self) -> Result<&mut dyn Driver> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Postgres(c) => Ok(c),
            #[cfg(feature = "sqlite")]
            Self::Sqlite(c) => Ok(c),
            #[cfg(not(any(feature = "postgres", feature = "sqlite")))]
            _ => Err(Fault::internal("no database feature enabled")),
        }
    }
    pub(crate) fn transaction<T>(
        &mut self,
        f: impl FnOnce(&mut dyn Driver) -> Result<T>,
    ) -> Result<T> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Postgres(c) => c.transaction(|db| f(db)),
            #[cfg(feature = "sqlite")]
            Self::Sqlite(c) => c.immediate_transaction(|db| f(db)),
            #[cfg(not(any(feature = "postgres", feature = "sqlite")))]
            _ => {
                let _ = f;
                Err(Fault::internal("no database feature enabled"))
            }
        }
    }
}
