use crate::{Fault, Result, Transaction, driver::Query, value};
use paydirt_model::{Entity, Field, quote};
use serde_json::{Value, json};
use std::collections::BTreeSet;

enum Join<'a> {
    Table {
        table: String,
        owner: &'static str,
        target: &'static str,
    },
    Foreign(&'a Field),
}

impl Transaction<'_> {
    fn join<'a>(&self, e: &Entity, f: &Field, target: &'a Entity) -> Result<Join<'a>> {
        if f.inverse.is_empty() {
            return Ok(Join::Table {
                table: quote(&e.edge_table(f)),
                owner: "owner",
                target: "target",
            });
        }
        let forward = target.field(&f.inverse).map_err(Fault::invalid)?;
        if forward.is_column() {
            return Ok(Join::Foreign(forward));
        }
        Ok(Join::Table {
            table: quote(&target.edge_table(forward)),
            owner: "target",
            target: "owner",
        })
    }
    fn linked(&mut self, e: &Entity, f: &Field, owner_id: &Value) -> Result<Vec<Value>> {
        let target = self.model.entity(&f.target).map_err(Fault::invalid)?;
        let mut q = Query::new("");
        let mut conditions = self.wall(target, &mut q)?;
        let p = q.param(
            value::bind(e.key().map_err(Fault::internal)?, owner_id)?,
            self.db.dialect(),
        );
        let key = quote(&target.key().map_err(Fault::internal)?.column());
        conditions.push(match self.join(e, f, target)? {
            Join::Foreign(k) => format!("r.{}={p}", quote(&k.column())),
            Join::Table {
                table,
                owner,
                target,
            } => format!("r.{key} IN (SELECT {target} FROM {table} WHERE {owner}={p})"),
        });
        q.sql = format!(
            "SELECT {} AS value FROM {} AS r WHERE {} ORDER BY r.{key}",
            self.project_sql(target)?,
            quote(&target.table()),
            conditions.join(" AND ")
        );
        self.db
            .rows(q)?
            .iter()
            .map(|r| self.decode_row(target, r))
            .collect()
    }
    pub(crate) fn edge_image(
        &mut self,
        e: &Entity,
        f: &Field,
        owner: &Value,
        select: &Value,
    ) -> Result<Value> {
        let target = self.model.entity(&f.target).map_err(Fault::invalid)?;
        let rows = self.linked(e, f, owner)?;
        let key = target.key().map_err(Fault::internal)?;
        let mut images = rows
            .into_iter()
            .map(|row| {
                if select.is_object() {
                    self.image(target, row, select)
                } else {
                    Ok(json!({&key.json:row[&key.json]}))
                }
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(if f.collection.is_empty() {
            images.pop().unwrap_or(Value::Null)
        } else {
            Value::Array(images)
        })
    }
    pub(crate) fn detach_links(&mut self, e: &Entity, key: &Value) -> Result<()> {
        for parent in &self.model.entities {
            for f in parent
                .fields
                .iter()
                .filter(|f| !f.is_column() && f.inverse.is_empty())
            {
                let mut cols = Vec::new();
                if parent.name == e.name {
                    cols.push("owner");
                }
                if f.target == e.name {
                    cols.push("target");
                }
                if cols.is_empty() {
                    continue;
                }
                let mut q = Query::new("");
                let p = q.param(
                    value::bind(e.key().map_err(Fault::internal)?, key)?,
                    self.db.dialect(),
                );
                let condition = cols
                    .into_iter()
                    .map(|c| format!("{c}={p}"))
                    .collect::<Vec<_>>()
                    .join(" OR ");
                q.sql = format!(
                    "DELETE FROM {} WHERE {condition}",
                    quote(&parent.edge_table(f))
                );
                self.db.exec(q)?;
            }
        }
        Ok(())
    }
    pub(crate) fn write_edges(
        &mut self,
        e: &Entity,
        owner: &Value,
        req: &Value,
        creating: bool,
    ) -> Result<bool> {
        let mut changed = false;
        for f in e
            .fields
            .iter()
            .filter(|f| !f.is_column() && (creating || !f.immutable))
        {
            let clear = req[format!("{}Null", f.json)] == true;
            let Some(refs) = req.get(&f.json).or_else(|| clear.then_some(&Value::Null)) else {
                continue;
            };
            let target = self.model.entity(&f.target).map_err(Fault::invalid)?;
            let key = target.key().map_err(Fault::invalid)?;
            let refs = if clear {
                vec![]
            } else if f.collection.is_empty() {
                vec![refs]
            } else {
                refs.as_array()
                    .ok_or_else(|| Fault::invalid("expected edge references"))?
                    .iter()
                    .collect()
            };
            if !creating && f.collection == "list" && refs.is_empty() && !clear {
                continue;
            }
            let mut want = BTreeSet::new();
            for reference in refs {
                let row = self.lookup(target, reference, false)?;
                want.insert(row[&key.json].to_string());
            }
            let have = self
                .linked(e, f, owner)?
                .into_iter()
                .map(|r| r[&key.json].to_string())
                .collect::<BTreeSet<_>>();
            for (insert, values) in [
                (false, have.difference(&want).collect::<Vec<_>>()),
                (true, want.difference(&have).collect()),
            ] {
                for v in values {
                    let mut q = Query::new("");
                    let a = q.param(
                        value::bind(e.key().map_err(Fault::invalid)?, owner)?,
                        self.db.dialect(),
                    );
                    let b = q.param(
                        value::bind(
                            key,
                            &serde_json::from_str(v)
                                .map_err(|_| Fault::internal("invalid stored key"))?,
                        )?,
                        self.db.dialect(),
                    );
                    q.sql = match self.join(e, f, target)? {
                        Join::Table {
                            table,
                            owner,
                            target,
                        } => {
                            if insert {
                                format!("INSERT INTO {table} ({owner},{target}) VALUES ({a},{b})")
                            } else {
                                format!("DELETE FROM {table} WHERE {owner}={a} AND {target}={b}")
                            }
                        }
                        Join::Foreign(edge) => {
                            if edge.immutable {
                                return Err(Fault::invalid("owning edge is immutable"));
                            }
                            let col = quote(&edge.column());
                            let value = if insert {
                                a.clone()
                            } else if edge.nullable {
                                "NULL".into()
                            } else {
                                return Err(Fault::precondition("cannot clear a required edge"));
                            };
                            let where_ = if insert {
                                format!("({col} IS NULL OR {col}={a})")
                            } else {
                                format!("{col}={a}")
                            };
                            format!(
                                "UPDATE {} SET {col}={value} WHERE {}={b} AND {where_}",
                                quote(&target.table()),
                                quote(&key.column())
                            )
                        }
                    };
                    if self.db.exec(q)? != 1 {
                        return Err(Fault::precondition(
                            "edge changed concurrently or belongs to another owner",
                        ));
                    }
                    changed = true;
                }
            }
        }
        Ok(changed)
    }
}
