//! Private requests between the Robot service and the database owner.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Row {
    pub id: Vec<u8>,
    pub alias: String,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Key {
    Id(Vec<u8>),
    Alias(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Operation {
    Add {
        rows: Vec<Row>,
        action: String,
    },
    Patch {
        key: Key,
        alias: String,
        now: i64,
    },
    Erase(Key),
    Get(Key),
    List {
        after: Option<(i64, Vec<u8>)>,
        limit: i64,
        keys: Option<Vec<Key>>,
    },
    Snapshot {
        keys: Vec<Key>,
    },
    Changes {
        since: i64,
        ids: Vec<Vec<u8>>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub tenant: Vec<u8>,
    pub op: Operation,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Change {
    pub id: Vec<u8>,
    pub value: Option<Row>,
    pub action: String,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Batch {
    pub rows: Vec<Row>,
    pub revision: i64,
    pub changes: Vec<Change>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Failure {
    Conflict,
    Missing,
    Unavailable,
    Internal,
}
pub type Reply = Result<Batch, Failure>;
