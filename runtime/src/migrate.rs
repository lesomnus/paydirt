use crate::{
    Fault, Result,
    driver::{Bind, Driver, Query},
};
use paydirt_model::{Dialect, Model, quote};

fn record(db: &mut dyn Driver, id: i32, text: String) -> Result<()> {
    let mut q = Query::new("");
    let p = q.param(Bind::Text(text), db.dialect());
    q.sql = format!(
        "INSERT INTO paydirt_schema (id,model) VALUES ({id},{p}) ON CONFLICT (id) DO UPDATE SET model=excluded.model"
    );
    db.exec(q)?;
    Ok(())
}
fn fingerprint(db: &mut dyn Driver, model: &Model) -> Result<String> {
    let mut structure = Vec::new();
    for name in model.entities.iter().flat_map(|e| e.managed_tables()) {
        let mut q = Query::new("");
        let table = q.param(Bind::Text(name.clone()), db.dialect());
        q.sql=match db.dialect() {
            Dialect::Sqlite=>format!("SELECT type || ':' || name || ':' || coalesce(sql,'') AS value FROM sqlite_master WHERE tbl_name={table} ORDER BY type,name"),
            Dialect::Postgres=>format!("WITH t AS (SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=current_schema() AND c.relname={table} AND c.relkind='r')
              SELECT value FROM (
              SELECT 'column:' || a.attname || ':' || format_type(a.atttypid,a.atttypmod) || ':' || a.attnotnull::text || ':' || coalesce(pg_get_expr(d.adbin,d.adrelid),'') AS value FROM t JOIN pg_attribute a ON a.attrelid=t.oid LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum WHERE a.attnum>0 AND NOT a.attisdropped
              UNION ALL SELECT 'constraint:' || pg_get_constraintdef(k.oid) FROM t JOIN pg_constraint k ON k.conrelid=t.oid
              UNION ALL SELECT 'index:' || pg_get_indexdef(i.indexrelid) FROM t JOIN pg_index i ON i.indrelid=t.oid) AS layout ORDER BY value"),
        };
        let rows = db.rows(q)?;
        if rows.is_empty() {
            return Err(Fault::precondition(format!("missing managed table {name}")));
        }
        structure.push((name, rows));
    }
    serde_json::to_string(&structure).map_err(|e| Fault::internal(e.to_string()))
}
pub(crate) fn install(db: &mut dyn Driver, model: &Model) -> Result<()> {
    if db.dialect() == Dialect::Postgres {
        db.batch("SELECT pg_advisory_xact_lock(7174429307262620)")?
    }
    db.batch(
        "CREATE TABLE IF NOT EXISTS paydirt_schema (id INTEGER PRIMARY KEY, model TEXT NOT NULL)",
    )?;
    let rows = db.rows(Query::new(
        "SELECT model AS value FROM paydirt_schema WHERE id IN (1,2) ORDER BY id",
    ))?;
    let old = rows
        .first()
        .map(|s| Model::parse(s.as_bytes()).map_err(Fault::precondition))
        .transpose()?;
    if let Some(old) = &old {
        let actual = fingerprint(db, old)?;
        if let Some(stored) = rows.get(1) {
            if stored != &actual {
                return Err(Fault::precondition(
                    "physical database schema has drifted from the recorded layout",
                ));
            }
            if model == old {
                return Ok(());
            }
        } else {
            // The original store recorded only the model. Verify its expected columns
            // before enrolling the existing database in physical drift checks.
            for e in &old.entities {
                let columns = e
                    .fields
                    .iter()
                    .filter(|f| f.is_column())
                    .map(|f| quote(&f.column()))
                    .collect::<Vec<_>>()
                    .join(", ");
                db.batch(&format!(
                    "SELECT {columns} FROM {} LIMIT 0",
                    quote(&e.table())
                ))
                .map_err(|_| {
                    Fault::precondition(format!(
                        "table {} does not match its stored model",
                        e.table()
                    ))
                })?;
            }
        }
    }
    let changes = model
        .migration(old.as_ref(), db.dialect())
        .map_err(Fault::precondition)?;
    if !changes.is_empty() {
        db.batch(&changes)?
    }
    record(
        db,
        1,
        serde_json::to_string(model).map_err(|e| Fault::internal(e.to_string()))?,
    )?;
    let layout = fingerprint(db, model)?;
    record(db, 2, layout)
}
