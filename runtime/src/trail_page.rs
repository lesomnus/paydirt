//! Ordered reads for trail maintenance and audit verification.
use crate::{
    Fault, Result, Scope, Transaction,
    driver::{Bind, Query},
    value,
};
use chrono::{DateTime, Utc};
use paydirt_model::{Entity, quote};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Whose {
    #[default]
    Anyone,
    FiledUnder,
    Alone,
    Together,
    Sharing,
    Named,
}
#[derive(Default)]
pub struct Selection {
    pub whose: Whose,
    pub tenant: [u8; 16],
    pub only: BTreeSet<u8>,
    pub except: BTreeSet<u8>,
    pub objects: BTreeSet<[u8; 16]>,
}
impl Selection {
    fn includes(&self, domain: u8) -> bool {
        if self.only.is_empty() {
            !self.except.contains(&domain)
        } else {
            self.only.contains(&domain)
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cursor {
    pub created: DateTime<Utc>,
    pub key: [u8; 16],
}
#[derive(Clone, Copy, Default)]
pub(crate) enum ReadMode {
    #[default]
    Oldest,
    ForUpdate,
    Latest,
}
#[derive(Clone, Copy)]
pub(crate) struct Page {
    pub before: Option<DateTime<Utc>>,
    pub after: Option<Cursor>,
    pub limit: usize,
    pub mode: ReadMode,
}
pub struct Row {
    pub cursor: Cursor,
    pub tenants: Vec<[u8; 16]>,
    pub entity: String,
    pub document: Value,
}
fn invalid(v: impl ToString) -> Fault {
    Fault::invalid(v.to_string())
}
fn column(e: &Entity, name: &str) -> Result<String> {
    Ok(format!(
        "r.{}",
        quote(&e.field(name).map_err(invalid)?.column())
    ))
}
fn key(row: &Value) -> Option<[u8; 16]> {
    value::bytes(row).ok()?.try_into().ok()
}
fn stamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}
fn kinds(q: &mut Query, d: paydirt_model::Dialect, values: &BTreeSet<u8>) -> String {
    values
        .iter()
        .map(|n| q.param(Bind::Big(i64::from(*n)), d))
        .collect::<Vec<_>>()
        .join(",")
}
impl Transaction<'_> {
    fn guard_maintenance(&self) -> Result<()> {
        if !matches!(self.scope, Scope::Internal) {
            return Err(Fault::denied(
                "trail maintenance requires a host transaction",
            ));
        }
        Ok(())
    }
    pub(crate) fn seek_query(
        &self,
        e: &Entity,
        kind: &str,
        select: &Selection,
        page: Page,
    ) -> Result<Query> {
        let Page {
            before,
            after,
            limit,
            mode,
        } = page;
        let d = self.db.dialect();
        let mut q = Query::new("");
        let mut terms = Vec::new();
        let time = column(e, "date_created")?;
        let id = column(e, &e.key)?;
        if kind != "OWN_OWED" {
            if kind == "OWN_AUDIT" {
                let domain = column(e, "domain")?;
                if !select.only.is_empty() {
                    terms.push(format!("{domain} IN ({})", kinds(&mut q, d, &select.only)));
                } else if !select.except.is_empty() {
                    terms.push(format!(
                        "{domain} NOT IN ({})",
                        kinds(&mut q, d, &select.except)
                    ));
                }
                if !select.objects.is_empty() {
                    let ids = select
                        .objects
                        .iter()
                        .map(|id| q.param(Bind::Bytes(id.to_vec()), d))
                        .collect::<Vec<_>>();
                    terms.push(format!(
                        "{} IN ({})",
                        column(e, "object_id")?,
                        ids.join(",")
                    ));
                }
            }
            if select.whose != Whose::Anyone {
                let tenant = q.param(Bind::Bytes(select.tenant.to_vec()), d);
                let filed = format!("{}={tenant}", column(e, "tenant_id")?);
                if select.whose == Whose::FiledUnder {
                    terms.push(filed);
                } else {
                    let owners = ["actor_tenant_id", "counterpart_tenant_id"]
                        .into_iter()
                        .filter(|n| e.field(n).is_ok())
                        .map(|n| column(e, n))
                        .collect::<Result<Vec<_>>>()?;
                    let mut any = vec![filed.clone()];
                    any.extend(owners.iter().map(|c| format!("{c}={tenant}")));
                    let any = format!("({})", any.join(" OR "));
                    if select.whose == Whose::Named {
                        terms.push(any);
                    } else {
                        let zero = q.param(Bind::Bytes(vec![0; 16]), d);
                        let mut alone = vec![filed.clone()];
                        alone.extend(
                            owners
                                .iter()
                                .map(|c| format!("({c} IS NULL OR {c} IN ({tenant},{zero}))")),
                        );
                        let alone = format!("({})", alone.join(" AND "));
                        terms.push(match select.whose {
                            Whose::Alone => alone,
                            Whose::Together => format!("{any} AND NOT {alone}"),
                            Whose::Sharing => format!("{filed} AND NOT {alone}"),
                            _ => unreachable!(),
                        });
                    }
                }
            }
        }
        if let Some(before) = before {
            let end = q.param(Bind::Text(stamp(before)), d);
            terms.push(format!("{time}<{end}"));
        }
        if let Some(cursor) = after {
            let at = q.param(Bind::Text(stamp(cursor.created)), d);
            let key = q.param(Bind::Bytes(cursor.key.to_vec()), d);
            terms.push(format!("({time},{id})>({at},{key})"));
        }
        let predicate = if terms.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", terms.join(" AND "))
        };
        let order = if matches!(mode, ReadMode::Latest) {
            " DESC"
        } else {
            ""
        };
        q.sql = format!(
            "SELECT {} AS value FROM {} r{predicate} ORDER BY {time}{order},{id}{order} LIMIT {limit}",
            self.project_sql(e)?,
            quote(&e.table())
        );
        if matches!(mode, ReadMode::ForUpdate) && d == paydirt_model::Dialect::Postgres {
            q.sql.push_str(" FOR UPDATE");
        }
        Ok(q)
    }
    /// Reads audit and access documents in a common (created, identifier) order.
    pub fn trail_page(
        &mut self,
        select: &Selection,
        before: Option<DateTime<Utc>>,
        after: Option<Cursor>,
        limit: usize,
    ) -> Result<Vec<Row>> {
        self.read_trail_page(select, before, after, limit, false, false)
    }
    pub fn owed_page(&mut self, after: Option<Cursor>, limit: usize) -> Result<Vec<Row>> {
        self.read_trail_page(&Selection::default(), None, after, limit, true, false)
    }
    pub(crate) fn read_trail_page(
        &mut self,
        select: &Selection,
        before: Option<DateTime<Utc>>,
        after: Option<Cursor>,
        limit: usize,
        owed: bool,
        lock: bool,
    ) -> Result<Vec<Row>> {
        self.guard_maintenance()?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        if i64::try_from(limit).is_err() {
            return Err(invalid("invalid trail page size"));
        }
        let mut result = Vec::new();
        for (name, p) in self.policies {
            let Some(kind) = p.entity["own"].as_str() else {
                continue;
            };
            if if owed {
                kind != "OWN_OWED"
            } else {
                !matches!(kind, "OWN_AUDIT" | "OWN_ACCESS")
            } {
                continue;
            }
            if kind == "OWN_ACCESS"
                && (!select.objects.is_empty()
                    || !p.entity["domain"]
                        .as_u64()
                        .and_then(|v| u8::try_from(v).ok())
                        .is_some_and(|d| select.includes(d)))
            {
                continue;
            }
            let e = self.model.entity(name).map_err(invalid)?;
            let query = self.seek_query(
                e,
                kind,
                select,
                Page {
                    before,
                    after,
                    limit,
                    mode: if lock {
                        ReadMode::ForUpdate
                    } else {
                        ReadMode::Oldest
                    },
                },
            )?;
            for raw in self.db.rows(query)? {
                let document = self.decode_row(e, &raw)?;
                let (created, key_id) = crate::integrity::order(&document)?;
                let mut tenants = Vec::new();
                for field in ["tenantId", "actorTenantId", "counterpartTenantId"] {
                    if let Some(id) = key(&document[field])
                        && id != [0; 16]
                        && !tenants.contains(&id)
                    {
                        tenants.push(id);
                    }
                }
                result.push(Row {
                    cursor: Cursor {
                        created,
                        key: key_id,
                    },
                    tenants,
                    entity: name.clone(),
                    document,
                });
            }
        }
        result.sort_by_key(|v| v.cursor);
        result.truncate(limit);
        Ok(result)
    }
    /// Each seek advances to the next filing tenant in either source index.
    pub fn trail_tenants(&mut self, mut after: [u8; 16], limit: usize) -> Result<Vec<[u8; 16]>> {
        self.guard_maintenance()?;
        let mut tenants = Vec::new();
        while tenants.len() < limit {
            let mut candidates = Vec::new();
            for (name, p) in self.policies {
                if !matches!(p.entity["own"].as_str(), Some("OWN_AUDIT" | "OWN_ACCESS")) {
                    continue;
                }
                let e = self.model.entity(name).map_err(invalid)?;
                let c = column(e, "tenant_id")?;
                let mut q = Query::new("");
                let bound = q.param(Bind::Bytes(after.to_vec()), self.db.dialect());
                let hex = match self.db.dialect() {
                    paydirt_model::Dialect::Postgres => format!("encode({c},'hex')"),
                    paydirt_model::Dialect::Sqlite => format!("hex({c})"),
                };
                q.sql = format!(
                    "SELECT {hex} AS value FROM {} r WHERE {c}>{bound} ORDER BY {c} LIMIT 1",
                    quote(&e.table())
                );
                if let Some(raw) = self.db.rows(q)?.first() {
                    let id: [u8; 16] = hex::decode(raw)
                        .map_err(invalid)?
                        .try_into()
                        .map_err(|_| invalid("invalid filing tenant"))?;
                    candidates.push(id);
                }
            }
            let Some(next) = candidates.into_iter().min() else {
                break;
            };
            tenants.push(next);
            after = next;
        }
        Ok(tenants)
    }
}

#[cfg(all(test, feature = "postgres", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::{Scope, Store};
    use diesel::{Connection, connection::SimpleConnection};
    use serde_json::json;

    // EXPLAIN cannot be used as a SQL subquery. The temporary function
    // returns the real query plan through the store's ordinary text driver.
    const EXPLAIN: &str = r#"
        CREATE FUNCTION pg_temp.paydirt_plan(command text) RETURNS text LANGUAGE plpgsql AS $$
        DECLARE plan json;
        BEGIN
            EXECUTE 'EXPLAIN (ANALYZE, FORMAT JSON) ' || command INTO plan;
            RETURN plan::text;
        END $$;
        SET LOCAL enable_seqscan=off;
        SET LOCAL enable_bitmapscan=off;
        SET LOCAL enable_sort=off;
    "#;
    fn explain(tx: &mut Transaction<'_>, mut query: Query, table: &str, limit: usize) {
        for (i, arg) in query.args.into_iter().enumerate().rev() {
            let lit = match arg {
                Bind::Big(n) => n.to_string(),
                Bind::Bytes(bytes) => format!("decode('{}','hex')", hex::encode(bytes)),
                Bind::Text(s) => format!("'{}'", s.replace('\'', "''")),
                _ => panic!("unexpected plan input"),
            };
            query.sql = query.sql.replace(&format!("${}", i + 1), &lit);
        }
        let mut run = Query::new("");
        let input = run.param(
            Bind::Text(query.sql.clone()),
            paydirt_model::Dialect::Postgres,
        );
        run.sql = format!("SELECT pg_temp.paydirt_plan({input}) AS value");
        let raw = tx.db.rows(run).unwrap();
        let plan: Value = serde_json::from_str(&raw[0]).unwrap();
        let mut nodes = vec![&plan[0]["Plan"]];
        let mut reads = 0;
        while let Some(node) = nodes.pop() {
            if node["Relation Name"] == table {
                reads += 1;
                assert!(
                    node["Node Type"].as_str().unwrap().contains("Index"),
                    "{node}"
                );
                assert!(
                    node["Actual Rows"].as_f64().unwrap() < (limit * 2) as f64,
                    "{}\n{node}",
                    query.sql
                );
                assert!(
                    node["Rows Removed by Filter"].as_f64().unwrap_or(0.0) < limit as f64,
                    "{}\n{node}",
                    query.sql
                );
            }
            if let Some(v) = node["Plans"].as_array() {
                nodes.extend(v);
            }
        }
        assert_eq!(reads, 1, "{plan}");
        assert!(plan[0]["Plan"]["Actual Rows"].as_u64().unwrap() > 0);
    }
    #[test]
    fn postgres_scans_begin_at_tuple_cursor_and_stop_at_page() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let mut admin = diesel::PgConnection::establish(&url).unwrap();
        let schema = format!("pageplan_{}", hex::encode(paydirt::mint(0)));
        admin
            .batch_execute(&format!("CREATE SCHEMA {schema}"))
            .unwrap();
        let scoped = format!(
            "{url}{}options=-csearch_path%3D{schema}",
            if url.contains('?') { "&" } else { "?" }
        );
        let result = std::panic::catch_unwind(|| {
            let mut db = Store::open(
                &scoped,
                include_str!("../../generated/storage/model.json"),
                include_str!("../../generated/storage/policies.json"),
            )
            .unwrap();
            let tenant = paydirt::mint(1);
            let other = paydirt::mint(1);
            let start = "2020-02-02T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
            let mut rows = Vec::new();
            let mut mid = None;
            for i in 0..1200 {
                let created = start + chrono::TimeDelta::seconds(i / 3);
                let t = if i % 3 == 0 { other } else { tenant };
                let id = paydirt::mint(3);
                if i == 600 {
                    mid = Some(Cursor { created, key: id });
                }
                let domain = if i % 16 == 0 { 2 } else { 10 };
                rows.push(("app.Audit".into(),json!({"id":value::encoded(&id),"tenantId":value::encoded(&t),
                    "actorTenantId":value::encoded(&t),"actorId":value::encoded(&[0;16]),"objectId":value::encoded(&paydirt::mint(domain)),"domain":domain,"action":"/test/Write","dateCreated":created})));
                rows.push(("app.Access".into(),json!({"id":value::encoded(&paydirt::mint(6)),"tenantId":value::encoded(&t),"actorTenantId":value::encoded(&t),
                    "actorId":value::encoded(&[0;16]),"domain":6,"action":"/test/Get","dateCreated":created})));
                rows.push(("app.Owed".into(),json!({"id":value::encoded(&paydirt::mint(254)),"object":value::encoded(&paydirt::mint(10)),"heldBy":[],"dateCreated":created})));
            }
            db.seed(rows).unwrap();
            db.transaction(&Scope::Internal, |tx| {
                tx.db.batch(EXPLAIN)?;
                for (name, kind, whose, only, except, limit) in [
                    ("app.Audit", "OWN_AUDIT", Whose::Anyone, Some(10), None, 256),
                    (
                        "app.Audit",
                        "OWN_AUDIT",
                        Whose::FiledUnder,
                        Some(10),
                        None,
                        50,
                    ),
                    ("app.Audit", "OWN_AUDIT", Whose::Anyone, None, Some(2), 50),
                    ("app.Audit", "OWN_AUDIT", Whose::Anyone, None, None, 50),
                    ("app.Access", "OWN_ACCESS", Whose::Anyone, None, None, 50),
                    (
                        "app.Access",
                        "OWN_ACCESS",
                        Whose::FiledUnder,
                        None,
                        None,
                        50,
                    ),
                    ("app.Owed", "OWN_OWED", Whose::Anyone, None, None, 50),
                ] {
                    let selection = Selection {
                        tenant,
                        whose,
                        only: only.into_iter().collect(),
                        except: except.into_iter().collect(),
                        ..Default::default()
                    };
                    let entity = tx.model.entity(name).unwrap();
                    let q = tx.seek_query(
                        entity,
                        kind,
                        &selection,
                        Page {
                            before: Some(start + chrono::TimeDelta::days(1)),
                            after: mid,
                            limit,
                            mode: ReadMode::Oldest,
                        },
                    )?;
                    explain(tx, q, &entity.table(), limit);
                }
                Ok(())
            })
            .unwrap();
        });
        admin
            .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
            .unwrap();
        if let Err(err) = result {
            std::panic::resume_unwind(err);
        }
    }
}
