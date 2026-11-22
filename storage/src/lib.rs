use diesel::{
    connection::SimpleConnection,
    prelude::*,
    result::{DatabaseErrorKind, Error},
};
use paydirt_robot_protocol::{Batch, Change, Failure, Key, Operation, Reply, Request, Row};

#[cfg(target_arch = "wasm32")]
type Db = SqliteConnection;
#[cfg(not(target_arch = "wasm32"))]
type Db = PgConnection;

diesel::table! { paydirt_robots (id) { id -> Binary, tenant -> Binary, alias -> Text, created_ms -> BigInt, updated_ms -> BigInt, erased -> Bool, } }
diesel::table! { paydirt_revisions (tenant) { tenant -> Binary, revision -> BigInt, } }
diesel::table! { paydirt_changes (tenant, revision) { tenant -> Binary, revision -> BigInt, robot_id -> Binary, payload -> Text, } }
use paydirt_changes::dsl as c;
use paydirt_revisions::dsl as v;
use paydirt_robots::dsl as r;

fn failure(err: Error) -> Failure {
    match err {
        Error::DatabaseError(DatabaseErrorKind::UniqueViolation, _) => Failure::Conflict,
        Error::NotFound => Failure::Missing,
        _ => Failure::Internal,
    }
}

pub struct Store(Db);

impl Store {
    pub fn open(url: &str) -> Result<Self, String> {
        let mut conn = Db::establish(url).map_err(|e| e.to_string())?;
        conn.batch_execute(include_str!("../schema.sql"))
            .map_err(|e| e.to_string())?;
        Ok(Self(conn))
    }

    pub fn execute(&mut self, req: Request) -> Reply {
        self.0.transaction(|db| execute(db, req)).map_err(failure)
    }
}

fn read_rows(
    db: &mut Db,
    tenant: &[u8],
    keys: Option<Vec<Key>>,
    after: Option<(i64, Vec<u8>)>,
    limit: i64,
) -> QueryResult<Vec<Row>> {
    let mut query = r::paydirt_robots
        .filter(r::tenant.eq(tenant))
        .filter(r::erased.eq(false))
        .into_boxed();
    if let Some(keys) = keys {
        let mut ids = Vec::new();
        let mut names = Vec::new();
        for key in keys {
            match key {
                Key::Id(id) => ids.push(id),
                Key::Alias(alias) => names.push(alias),
            }
        }
        query = query.filter(r::id.eq_any(ids).or(r::alias.eq_any(names)));
    }
    if let Some((time, id)) = after {
        query = query.filter(
            r::created_ms
                .gt(time)
                .or(r::created_ms.eq(time).and(r::id.gt(id))),
        );
    }
    query
        .order((r::created_ms.asc(), r::id.asc()))
        .limit(limit)
        .select((r::id, r::alias, r::created_ms, r::updated_ms))
        .load::<(Vec<u8>, String, i64, i64)>(db)
        .map(|rows| {
            rows.into_iter()
                .map(|(id, alias, created_ms, updated_ms)| Row {
                    id,
                    alias,
                    created_ms,
                    updated_ms,
                })
                .collect()
        })
}

fn get(db: &mut Db, tenant: &[u8], key: Key) -> QueryResult<Row> {
    read_rows(db, tenant, Some(vec![key]), None, 1)?
        .pop()
        .ok_or(Error::NotFound)
}

// All writers take the tenant's row lock before reading a preimage or allocating a revision.
fn lock_tenant(db: &mut Db, tenant: &[u8]) -> QueryResult<()> {
    diesel::insert_into(v::paydirt_revisions)
        .values((v::tenant.eq(tenant), v::revision.eq(0i64)))
        .on_conflict(v::tenant)
        .do_nothing()
        .execute(db)?;
    diesel::update(v::paydirt_revisions.find(tenant))
        .set(v::revision.eq(v::revision))
        .execute(db)?;
    Ok(())
}

fn record(
    db: &mut Db,
    tenant: &[u8],
    id: &[u8],
    value: Option<Row>,
    action: &str,
) -> QueryResult<()> {
    diesel::update(v::paydirt_revisions.find(tenant))
        .set(v::revision.eq(v::revision + 1i64))
        .execute(db)?;
    let rev: i64 = v::paydirt_revisions
        .find(tenant)
        .select(v::revision)
        .first(db)?;
    let change = Change {
        id: id.to_vec(),
        value,
        action: action.into(),
    };
    let payload =
        serde_json::to_string(&change).map_err(|e| Error::SerializationError(Box::new(e)))?;
    diesel::insert_into(c::paydirt_changes)
        .values((
            c::tenant.eq(tenant),
            c::revision.eq(rev),
            c::robot_id.eq(id),
            c::payload.eq(payload),
        ))
        .execute(db)?;
    Ok(())
}

fn execute(db: &mut Db, req: Request) -> QueryResult<Batch> {
    let tenant = req.tenant;
    let mut batch = Batch::default();
    match req.op {
        Operation::Add { rows, action } => {
            lock_tenant(db, &tenant)?;
            for row in &rows {
                diesel::insert_into(r::paydirt_robots)
                    .values((
                        r::id.eq(&row.id),
                        r::tenant.eq(&tenant),
                        r::alias.eq(&row.alias),
                        r::created_ms.eq(row.created_ms),
                        r::updated_ms.eq(row.updated_ms),
                    ))
                    .execute(db)?;
                record(db, &tenant, &row.id, Some(row.clone()), &action)?;
            }
            batch.rows = rows;
        }
        Operation::Patch { key, alias, now } => {
            lock_tenant(db, &tenant)?;
            let mut row = get(db, &tenant, key)?;
            row.alias = alias;
            row.updated_ms = now.max(row.updated_ms + 1);
            diesel::update(
                r::paydirt_robots
                    .find(&row.id)
                    .filter(r::tenant.eq(&tenant)),
            )
            .set((r::alias.eq(&row.alias), r::updated_ms.eq(row.updated_ms)))
            .execute(db)?;
            record(
                db,
                &tenant,
                &row.id,
                Some(row.clone()),
                "/app.RobotService/Patch",
            )?;
            batch.rows.push(row);
        }
        Operation::Erase(key) => {
            lock_tenant(db, &tenant)?;
            if let Some(row) = get(db, &tenant, key).optional()? {
                diesel::update(
                    r::paydirt_robots
                        .find(&row.id)
                        .filter(r::tenant.eq(&tenant)),
                )
                .set(r::erased.eq(true))
                .execute(db)?;
                record(db, &tenant, &row.id, None, "/app.RobotService/Erase")?;
                batch.rows.push(row);
            }
        }
        Operation::Get(key) => batch.rows.push(get(db, &tenant, key)?),
        Operation::List { keys, after, limit } => {
            batch.rows = read_rows(db, &tenant, keys, after, limit)?
        }
        Operation::Snapshot { keys } => {
            // Alias resolution, rows and watermark share a snapshot. The service bounds keys at 32.
            #[cfg(not(target_arch = "wasm32"))]
            db.batch_execute("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")?;
            batch.revision = v::paydirt_revisions
                .find(&tenant)
                .select(v::revision)
                .first(db)
                .optional()?
                .unwrap_or(0);
            for key in keys {
                let row = get(db, &tenant, key)?;
                if !batch.rows.iter().any(|r| r.id == row.id) {
                    batch.rows.push(row);
                }
            }
            batch
                .rows
                .sort_by(|a, b| (a.created_ms, &a.id).cmp(&(b.created_ms, &b.id)));
        }
        Operation::Changes { since, ids } => {
            batch.revision = since;
            for (rev, payload) in c::paydirt_changes
                .filter(c::tenant.eq(&tenant))
                .filter(c::revision.gt(since))
                .filter(c::robot_id.eq_any(ids))
                .order(c::revision.asc())
                .limit(100)
                .select((c::revision, c::payload))
                .load::<(i64, String)>(db)?
            {
                batch.revision = rev;
                batch.changes.push(
                    serde_json::from_str(&payload)
                        .map_err(|e| Error::DeserializationError(Box::new(e)))?,
                );
            }
        }
    }
    Ok(batch)
}

#[cfg(target_arch = "wasm32")]
mod worker {
    use super::*;
    use sqlite_wasm_vfs::sahpool::{OpfsSAHPoolCfg, install};
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct Database(Store);
    #[wasm_bindgen]
    impl Database {
        pub async fn open() -> Result<Database, JsError> {
            install::<sqlite_wasm_rs::WasmOsCallback>(&OpfsSAHPoolCfg::default(), true)
                .await
                .map_err(|e| JsError::new(&e.to_string()))?;
            Store::open("paydirt.sqlite3")
                .map(Database)
                .map_err(|e| JsError::new(&e))
        }
        pub fn execute(&mut self, request: &str) -> Result<String, JsError> {
            let req = serde_json::from_str(request)?;
            Ok(serde_json::to_string(&self.0.execute(req))?)
        }
    }
}
