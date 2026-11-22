use crate::{
    Fault, Policies, Result,
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
              UNION ALL SELECT 'index:' || pg_get_indexdef(i.indexrelid) FROM t JOIN pg_index i ON i.indrelid=t.oid WHERE i.indisvalid AND i.indisready) AS layout ORDER BY value"),
        };
        let rows = db.rows(q)?;
        if rows.is_empty() {
            return Err(Fault::precondition(format!("missing managed table {name}")));
        }
        structure.push((name, rows));
    }
    serde_json::to_string(&structure).map_err(|e| Fault::internal(e.to_string()))
}
pub(crate) fn install(db: &mut dyn Driver, model: &Model, policies: &Policies) -> Result<()> {
    if db.dialect() == Dialect::Postgres {
        db.batch("SELECT pg_advisory_xact_lock(7174429307262620)")?
    }
    db.batch(
        "CREATE TABLE IF NOT EXISTS paydirt_schema (id INTEGER PRIMARY KEY, model TEXT NOT NULL)",
    )?;
    let rows = db.rows(Query::new(
        "SELECT model AS value FROM paydirt_schema WHERE id IN (1,2) ORDER BY id",
    ))?;
    let mut old = rows
        .first()
        .map(|s| Model::parse(s.as_bytes()).map_err(Fault::precondition))
        .transpose()?;
    validate(db, old.as_ref().unwrap_or(model))?;
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
    if let Some(prior) = &mut old {
        rename(db, prior, model, policies)?;
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

/// Rename persisted entities and their owning edge tables before comparing field
/// definitions. Both databases update incoming foreign keys on ALTER TABLE.
fn rename(db: &mut dyn Driver, old: &mut Model, model: &Model, policies: &Policies) -> Result<()> {
    use std::collections::BTreeMap;
    let mut replacements = BTreeMap::new();
    for entity in &model.entities {
        let Some(policy) = policies.get(&entity.name) else {
            continue;
        };
        let package = entity.name.rsplit_once('.').map(|(prefix, _)| prefix);
        for former in policy.entity["formerly"].as_array().into_iter().flatten() {
            let short = former["message"]
                .as_str()
                .ok_or_else(|| Fault::invalid("formerly requires a message"))?;
            let from = package.map_or_else(|| short.to_owned(), |p| format!("{p}.{short}"));
            if model.entity(&from).is_ok()
                || replacements
                    .insert(from.clone(), entity.name.clone())
                    .is_some()
            {
                return Err(Fault::precondition(format!(
                    "ambiguous former entity {from}"
                )));
            }
        }
    }
    let mut used = std::collections::BTreeSet::new();
    for e in &old.entities {
        if !used.insert(replacements.get(&e.name).unwrap_or(&e.name)) {
            return Err(Fault::precondition(format!(
                "multiple earlier entities would merge at {}",
                e.name
            )));
        }
    }
    for e in &mut old.entities {
        if let Some(new) = replacements.get(&e.name) {
            let old_tables = e.managed_tables();
            let indexes = e.indexes();
            for idx in &indexes {
                db.batch(&format!("DROP INDEX {}", quote(&e.index_name(idx))))?;
            }
            e.name.clone_from(new);
            for (before, after) in old_tables.into_iter().zip(e.managed_tables()) {
                db.batch(&format!(
                    "ALTER TABLE {} RENAME TO {}",
                    quote(&before),
                    quote(&after)
                ))?;
            }
            // The normal migration recreates all declared indexes under their current names.
            e.indexes.clear();
            for f in &mut e.fields {
                f.unique = false;
            }
        }
        for f in &mut e.fields {
            if let Some(to) = replacements.get(&f.target) {
                f.target.clone_from(to);
            }
        }
    }
    Ok(())
}
fn validate(db: &mut dyn Driver, model: &Model) -> Result<()> {
    use std::collections::BTreeSet;
    for e in &model.entities {
        let table = e.table();
        let mut q = Query::new("");
        let name = q.param(Bind::Text(table.clone()), db.dialect());
        q.sql = match db.dialect() {
            Dialect::Sqlite => format!(
                "SELECT name AS value FROM pragma_table_info({name}) WHERE \"notnull\"=1 AND dflt_value IS NULL AND pk=0"
            ),
            Dialect::Postgres => format!(
                "SELECT a.attname::text AS value FROM pg_attribute a JOIN pg_class t ON t.oid=a.attrelid JOIN pg_namespace n ON n.oid=t.relnamespace WHERE n.nspname=current_schema() AND t.relname={name} AND a.attnum>0 AND NOT a.attisdropped AND a.attnotnull AND NOT a.atthasdef AND a.attidentity='' AND a.attgenerated=''"
            ),
        };
        let columns: BTreeSet<_> = e
            .fields
            .iter()
            .filter(|f| f.is_column())
            .map(|f| f.column())
            .collect();
        for field in db.rows(q)? {
            if !columns.contains(&field) {
                return Err(Fault::precondition(format!(
                    "schema drifted: unrecognized required column {table}.{field} has no default; review ALTER TABLE {} DROP COLUMN {}",
                    quote(&table),
                    quote(&field)
                )));
            }
        }
        if db.dialect() == Dialect::Postgres {
            let mut q = Query::new("");
            let name = q.param(Bind::Text(table.clone()), db.dialect());
            q.sql = format!(
                "SELECT c.relname::text AS value FROM pg_index i JOIN pg_class t ON t.oid=i.indrelid JOIN pg_namespace n ON n.oid=t.relnamespace JOIN pg_class c ON c.oid=i.indexrelid WHERE n.nspname=current_schema() AND t.relname={name} AND (NOT i.indisvalid OR NOT i.indisready)"
            );
            let declared: BTreeSet<_> = e.indexes().iter().map(|idx| e.index_name(idx)).collect();
            for index in db.rows(q)? {
                if declared.contains(&index) {
                    return Err(Fault::precondition(format!(
                        "declared index {index} on {table} is invalid or unready; repair the index before starting"
                    )));
                }
                eprintln!("unmanaged invalid or unready index {index} on {table}");
            }
        }
    }
    Ok(())
}
