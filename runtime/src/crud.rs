use crate::{
    Fault, Result, Scope, Transaction,
    driver::{Bind, Query},
    value,
};
use paydirt_model::{Dialect, Entity, Field, Scalar, quote};
use serde_json::{Map, Value, json};
type Object = Map<String, Value>;

impl Transaction<'_> {
    pub(crate) fn field_type<'a>(&'a self, f: &'a Field) -> Result<&'a Field> {
        if f.target.is_empty() {
            return Ok(f);
        }
        self.model
            .entity(&f.target)
            .and_then(Entity::key)
            .map_err(Fault::invalid)
    }
    pub(crate) fn project_sql(&self, e: &Entity) -> Result<String> {
        self.project_at(e, "r")
    }
    fn project_at(&self, e: &Entity, alias: &str) -> Result<String> {
        let d = self.db.dialect();
        let mut cols = Vec::new();
        for f in e.fields.iter().filter(|f| f.is_column()) {
            let c = format!("{alias}.{}", quote(&f.column()));
            cols.push(
                if self.model.scalar(f).map_err(Fault::invalid)? == Scalar::Binary {
                    if d == Dialect::Postgres {
                        format!("encode({c},'hex')")
                    } else {
                        format!("CASE WHEN {c} IS NULL THEN NULL ELSE hex({c}) END")
                    }
                } else {
                    format!("CAST({c} AS TEXT)")
                },
            );
        }
        let cols = cols.join(",");
        Ok(if d == Dialect::Postgres {
            format!("CAST(json_build_array({cols}) AS TEXT)")
        } else {
            format!("json_array({cols})")
        })
    }
    pub(crate) fn decode_row(&self, e: &Entity, row: &str) -> Result<Value> {
        let values: Vec<Option<String>> =
            serde_json::from_str(row).map_err(|e| Fault::internal(e.to_string()))?;
        let mut out = Object::new();
        for (f, v) in e.fields.iter().filter(|f| f.is_column()).zip(values) {
            let Some(v) = v else { continue };
            let ty = self.field_type(f)?;
            let v = value::decode(ty, &v)?;
            if f.target.is_empty() {
                out.insert(f.json.clone(), v);
            } else {
                out.insert(f.json.clone(), json!({&ty.json:v}));
            }
        }
        Ok(Value::Object(out))
    }
    fn tenant_path(&self, e: &Entity, path: &str, alias: &str, depth: u32) -> Result<String> {
        let (name, rest) = path.split_once('.').unwrap_or((path, ""));
        let f = e.field(name).map_err(Fault::invalid)?;
        let col = format!("{alias}.{}", quote(&f.column()));
        if rest.is_empty() {
            return Ok(col);
        }
        let target = self.model.entity(&f.target).map_err(Fault::invalid)?;
        let next = format!("s{depth}");
        if rest == target.key {
            return Ok(col);
        }
        let nested = self.tenant_path(target, rest, &next, depth + 1)?;
        Ok(format!(
            "(SELECT {nested} FROM {} AS {next} WHERE {next}.{}={col})",
            quote(&target.table()),
            quote(&target.key().map_err(Fault::invalid)?.column())
        ))
    }
    pub(crate) fn wall(&self, e: &Entity, q: &mut Query) -> Result<Vec<String>> {
        self.wall_at(e, q, "r")
    }
    fn wall_at(&self, e: &Entity, q: &mut Query, alias: &str) -> Result<Vec<String>> {
        let mut parts = Vec::new();
        if let Some(f) = e.erased() {
            parts.push(format!("{alias}.{} IS NULL", quote(&f.column())))
        }
        if !matches!(self.scope, Scope::Internal) {
            for rule in self.scope_rules {
                for (name, v) in rule(&e.name, self.frame)? {
                    let field = e.field(&name).map_err(Fault::invalid)?;
                    if !field.is_column() {
                        return Err(Fault::invalid("scope rules require scalar columns"));
                    }
                    let c = format!("{alias}.{}", quote(&field.column()));
                    parts.push(if v.is_null() {
                        format!("{c} IS NULL")
                    } else {
                        let p =
                            q.param(value::bind(self.field_type(field)?, &v)?, self.db.dialect());
                        format!("{c}={p}")
                    });
                }
            }
        }
        if let Scope::Account { id, reads } = self.scope {
            parts.extend(reads.clauses(e, id, q, self.db.dialect())?);
            return Ok(parts);
        }
        if let Scope::Granted {
            sets: Some(sets), ..
        } = self.scope
        {
            let group = e
                .fields
                .iter()
                .find(|f| f.number == 3 && !f.target.is_empty() && f.is_column());
            let is_set = self.model.entities.iter().any(|entity| {
                entity
                    .fields
                    .iter()
                    .any(|f| f.number == 3 && f.target == e.name && f.is_column())
            });
            if let Some(f) = group.or_else(|| is_set.then(|| e.key().ok()).flatten()) {
                let ids: Vec<_> = sets
                    .iter()
                    .map(|id| q.param(Bind::Bytes(id.clone()), self.db.dialect()))
                    .collect();
                if ids.is_empty() {
                    parts.push("1=0".into());
                } else {
                    parts.push(format!(
                        "{alias}.{} IN ({})",
                        quote(&f.column()),
                        ids.join(",")
                    ));
                }
            }
        }
        let Some(ids) = self.scope.tenants() else {
            return Ok(parts);
        };
        let p = &self
            .policies
            .get(&e.name)
            .ok_or_else(|| Fault::denied("entity has no scope declaration"))?
            .entity;
        if p.get("global").is_some() {
            return Ok(parts);
        }
        let paths: Vec<&str> = if p.get("tenant").is_some() {
            vec![&e.key]
        } else if let Some(v) = p["tenanted"]["field"].as_array() {
            v.iter().filter_map(Value::as_str).collect()
        } else if let Some(v) = p["tenanted"]["stamp"]
            .as_str()
            .or_else(|| p["tenanted"]["via"].as_str())
        {
            vec![v]
        } else {
            vec![
                e.fields
                    .iter()
                    .find(|f| f.number == 2 && !f.target.is_empty())
                    .ok_or_else(|| Fault::denied("entity has no tenant path"))?
                    .name
                    .as_str(),
            ]
        };
        if paths.is_empty() {
            return Err(Fault::denied("empty tenant scope"));
        }
        let mut any = Vec::new();
        if ids.is_empty() {
            parts.push("1=0".into());
            return Ok(parts);
        }
        for path in paths {
            let name = self.tenant_path(e, path, alias, 0)?;
            let vs: Vec<_> = ids
                .iter()
                .map(|id| q.param(Bind::Bytes(id.clone()), self.db.dialect()))
                .collect();
            any.push(format!("{name} IN ({})", vs.join(",")));
        }
        parts.push(format!("({})", any.join(" OR ")));
        Ok(parts)
    }
    pub(crate) fn resolved(&mut self, f: &Field, input: &Value) -> Result<Bind> {
        if f.target.is_empty() {
            return value::bind(f, input);
        }
        let target = self.model.entity(&f.target).map_err(Fault::invalid)?;
        let row = self.reference_row(target, input)?;
        let key = target.key().map_err(Fault::invalid)?;
        value::bind(key, &row[&key.json])
    }
    pub(crate) fn key_parts<'a>(
        &self,
        e: &'a Entity,
        key: &'a Value,
    ) -> Result<Vec<(&'a Field, &'a Value)>> {
        let o = key
            .as_object()
            .filter(|o| o.len() == 1)
            .ok_or_else(|| Fault::invalid("expected exactly one row reference"))?;
        let (name, v) = o.iter().next().unwrap();
        if let Some(f) = e
            .fields
            .iter()
            .find(|f| f.json == *name && (f.name == e.key || f.unique))
        {
            return Ok(vec![(f, v)]);
        }
        let idx = e
            .indexes
            .iter()
            .find(|i| {
                i.unique
                    && !i.hidden
                    && (i.name == *name || heck::AsLowerCamelCase(&i.name).to_string() == *name)
            })
            .ok_or_else(|| Fault::invalid("unknown reference key"))?;
        let mut fields = Vec::new();
        for n in &idx.fields {
            let f = e.field(n).map_err(Fault::invalid)?;
            let part = if idx.fields.len() == 1 {
                v
            } else {
                v.get(&f.json)
                    .ok_or_else(|| Fault::invalid("incomplete composite key"))?
            };
            fields.push((f, part))
        }
        Ok(fields)
    }
    pub(crate) fn lookup(&mut self, e: &Entity, key: &Value, lock: bool) -> Result<Value> {
        let mut q = Query::new("");
        let mut where_ = self.wall(e, &mut q)?;
        for (f, v) in self.key_parts(e, key)? {
            let b = if f.name == "alias" && f.number == 4 && f.kind == "STRING" {
                Bind::Text(crate::naming::parse(
                    v.as_str().ok_or_else(|| Fault::invalid("expected alias"))?,
                )?)
            } else {
                self.resolved(f, v)?
            };
            let p = q.param(b, self.db.dialect());
            where_.push(format!("r.{}={p}", quote(&f.column())));
        }
        let suffix = if lock && self.db.dialect() == Dialect::Postgres {
            " FOR UPDATE"
        } else {
            ""
        };
        q.sql = format!(
            "SELECT {} AS value FROM {} AS r WHERE {}{suffix}",
            self.project_sql(e)?,
            quote(&e.table()),
            where_.join(" AND ")
        );
        let row = self
            .db
            .rows(q)?
            .into_iter()
            .next()
            .ok_or_else(Fault::missing)?;
        self.decode_row(e, &row)
    }
    pub(crate) fn image(&mut self, e: &Entity, mut row: Value, select: &Value) -> Result<Value> {
        let all = select.is_null() || select["all"] == true;
        let secrets = self.policies.get(&e.name);
        for f in &e.fields {
            if secrets
                .and_then(|p| p.fields.get(&f.name))
                .is_some_and(|p| p.get("secret").is_some() && p["secret"] != false)
                || (!all && f.name != e.key && select[&f.json].is_null())
                || (!select.is_null() && select[&f.json] == false && f.name != e.key)
            {
                row.as_object_mut().unwrap().remove(&f.json);
                continue;
            }
            if !f.is_column() {
                let image = self.edge_image(
                    e,
                    f,
                    &row[&e.key().map_err(Fault::invalid)?.json],
                    &select[&f.json],
                )?;
                if !image.is_null() {
                    row[&f.json] = image;
                }
                continue;
            }
            if !f.target.is_empty()
                && select[&f.json].is_object()
                && let Some(reference) = row.get(&f.json)
            {
                let target = self.model.entity(&f.target).map_err(Fault::invalid)?;
                let nested = self.lookup(target, reference, false)?;
                row[&f.json] = self.image(target, nested, &select[&f.json])?;
            }
        }
        Ok(row)
    }
    fn domain(&self, e: &Entity) -> Option<u8> {
        self.policies.get(&e.name)?.entity["domain"]
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
    }
    pub(crate) fn insert(&mut self, e: &Entity, req: &Value) -> Result<Value> {
        let mut rows = self.insert_items(e, std::slice::from_ref(req), false)?;
        rows.pop()
            .ok_or_else(|| Fault::internal("insert returned no row"))
    }
    pub(crate) fn item_error(mut err: Fault, index: usize, many: bool) -> Fault {
        if !many {
            return err;
        }
        let path = format!("items[{index}]");
        if err.code == "invalid_argument" {
            err.at(&path)
        } else {
            err.message = format!("{path}: {}", err.message);
            err
        }
    }
    fn insert_items(&mut self, e: &Entity, items: &[Value], many: bool) -> Result<Vec<Value>> {
        if items.is_empty() {
            return Ok(vec![]);
        }
        let alias = e
            .field("alias")
            .is_ok_and(|f| f.number == 4 && f.kind == "STRING");
        let generated = alias
            && items
                .iter()
                .any(|v| v["alias"].as_str().unwrap_or_default().is_empty());
        let mut conflict = String::new();
        if generated
            && let Some(index) = e
                .indexes()
                .into_iter()
                .find(|i| i.unique && i.fields.iter().any(|f| f == "alias"))
        {
            let names = index
                .fields
                .iter()
                .map(|name| e.field(name).map(|f| quote(&f.column())))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Fault::invalid)?;
            conflict = format!(" ON CONFLICT ({})", names.join(","));
            if index.active_only {
                let stamp = e
                    .erased()
                    .ok_or_else(|| Fault::internal("missing erase field"))?;
                conflict.push_str(&format!(" WHERE {} IS NULL", quote(&stamp.column())));
            }
            conflict.push_str(" DO NOTHING");
        }
        let retry_alias = !conflict.is_empty();
        let mut rows = items
            .iter()
            .enumerate()
            .map(|(i, v)| {
                self.admit(e, v)
                    .map_err(|err| Self::item_error(err, i, many))
            })
            .collect::<Result<Vec<_>>>()?;
        for attempt in 0..3 {
            if alias {
                for (i, (row, source)) in rows.iter_mut().zip(items).enumerate() {
                    let given = source["alias"].as_str().unwrap_or_default();
                    row["alias"] = json!(
                        self.namer
                            .name(crate::naming::NameRequest {
                                entity: &e.name,
                                given,
                                request: source,
                                frame: self.frame
                            })
                            .map_err(|mut err| {
                                if err.code == "invalid_argument"
                                    && err.field.is_none()
                                    && err.violations.is_empty()
                                {
                                    err.field = Some("alias".into());
                                }
                                Self::item_error(err, i, many)
                            })?
                    );
                }
            }
            // A name conflict rolls back the entire attempt, including earlier
            // parameter-limited chunks. Recorders run only for a complete batch.
            if retry_alias {
                self.db.batch("SAVEPOINT paydirt_names")?;
            }
            let result = self.insert_batch(e, &rows, many, &conflict);
            if retry_alias {
                if !matches!(&result, Ok(Some(_))) {
                    self.db.batch("ROLLBACK TO SAVEPOINT paydirt_names")?;
                }
                self.db.batch("RELEASE SAVEPOINT paydirt_names")?;
            }
            let inserted = match result? {
                Some(rows) => rows,
                None if attempt < 2 => continue,
                None => return Err(Fault::conflict("alias already exists")),
            };
            let mut answer = Vec::with_capacity(rows.len());
            for (i, (req, post)) in rows.iter().zip(inserted).enumerate() {
                let outcome = (|| {
                    self.write_edges(e, &post[&e.key().map_err(Fault::invalid)?.json], req, true)?;
                    self.record(e, "Add", req, None, Some(&post))?;
                    self.image(e, post, &Value::Null)
                })();
                answer.push(outcome.map_err(|err| Self::item_error(err, i, many))?);
            }
            return Ok(answer);
        }
        Err(Fault::conflict("key already exists"))
    }
    fn insert_batch(
        &mut self,
        e: &Entity,
        rows: &[Value],
        many: bool,
        conflict: &str,
    ) -> Result<Option<Vec<Value>>> {
        let columns: Vec<_> = e
            .fields
            .iter()
            .filter(|f| f.is_column() && !f.erased)
            .map(|f| quote(&f.column()))
            .collect();
        let table = quote(&e.table());
        let projection = self.project_at(e, &table)?;
        let mut answer = Vec::with_capacity(rows.len());
        let limit = match self.db.dialect() {
            Dialect::Postgres => 65000,
            Dialect::Sqlite => 30000,
        };
        let chunk_size = (limit / (columns.len().max(1) + 1)).max(1);
        for (page, chunk) in rows.chunks(chunk_size).enumerate() {
            let mut q = Query::new("");
            let mut keys = Vec::new();
            let mut tuples = Vec::new();
            for (offset, req) in chunk.iter().enumerate() {
                let (key, params) = self
                    .insert_params(e, req, &mut q)
                    .map_err(|err| Self::item_error(err, page * chunk_size + offset, many))?;
                keys.push(key.to_string());
                tuples.push(format!("({})", params.join(",")));
            }
            let predicates = self.wall_at(e, &mut q, &table)?;
            let visible = if predicates.is_empty() {
                projection.clone()
            } else {
                format!(
                    "CASE WHEN {} THEN {projection} ELSE 'null' END",
                    predicates.join(" AND ")
                )
            };
            q.sql = format!(
                "INSERT INTO {table} ({}) VALUES {}{conflict} RETURNING {visible} AS value",
                columns.join(","),
                tuples.join(",")
            );
            let mut result = std::collections::BTreeMap::new();
            let key = &e.key().map_err(Fault::invalid)?.json;
            for raw in self.db.rows(q)? {
                if raw == "null" {
                    return Err(Fault::missing());
                }
                let row = self.decode_row(e, &raw)?;
                let id = match value::bind(e.key().map_err(Fault::invalid)?, &row[key])? {
                    Bind::Bytes(bytes) => value::encoded(&bytes),
                    Bind::Big(i) => json!(i.to_string()),
                    Bind::Integer(i) => json!(i),
                    _ => row[key].clone(),
                };
                result.insert(id.to_string(), row);
            }
            if result.len() < chunk.len() && !conflict.is_empty() {
                return Ok(None);
            }
            for id in keys {
                answer.push(
                    result
                        .remove(&id)
                        .ok_or_else(|| Fault::internal("missing inserted row"))?,
                );
            }
        }
        Ok(Some(answer))
    }
    fn insert_params(
        &mut self,
        e: &Entity,
        req: &Value,
        q: &mut Query,
    ) -> Result<(Value, Vec<String>)> {
        let mut params = Vec::new();
        let mut id = Value::Null;
        for f in e.fields.iter().filter(|f| f.is_column()) {
            let supplied = req.get(&f.json).filter(|v| !v.is_null());
            let v = if f.erased {
                continue;
            } else if f.version {
                self.inputs.stamp()
            } else if let Some(v) = supplied {
                v.clone()
            } else if f.name == e.key
                && let Some(domain) = self.domain(e)
            {
                value::encoded(&(self.inputs.mint)(domain))
            } else if f.nullable {
                params.push("NULL".into());
                continue;
            } else if !f.target.is_empty() {
                return Err(Fault::invalid(format!("{} required", f.name)));
            } else if f.name == e.key {
                return Err(Fault::invalid("primary key required"));
            } else if let Some(def) = &f.default
                && !def.is_empty()
            {
                serde_json::from_str(def).unwrap_or_else(|_| json!(def))
            } else {
                value::default_value(f, self.inputs)
            };
            if f.name == e.key {
                if let Some(domain) = self.domain(e)
                    && !paydirt::has_domain(&value::bytes(&v)?, domain)
                {
                    crate::domain::Domains::from_policies(self.policies)?
                        .check_id(&value::bytes(&v)?, domain)?;
                }
                id = match value::bind(f, &v)? {
                    Bind::Bytes(bytes) => value::encoded(&bytes),
                    Bind::Big(i) => json!(i.to_string()),
                    Bind::Integer(i) => json!(i),
                    _ => v.clone(),
                };
            }
            let b = self.resolved(f, &v)?;
            params.push(q.param(b, self.db.dialect()));
        }
        Ok((id, params))
    }
    pub(crate) fn patch(&mut self, e: &Entity, req: &Value, method: &str) -> Result<Value> {
        self.check_membership_input(e, req)?;
        let mut req = req.clone();
        if e.field("alias")
            .is_ok_and(|f| f.number == 4 && f.kind == "STRING")
            && let Some(given) = req["alias"].as_str()
        {
            req["alias"] = json!(crate::naming::parse(given)?);
        }
        let req = &req;
        let row = self.lookup(e, &req["ref"], true)?;
        let mut q = Query::new("");
        let mut sets = Vec::new();
        for f in &e.fields {
            if self
                .policies
                .get(&e.name)
                .and_then(|p| p.entity["tenanted"]["stamp"].as_str())
                == Some(f.name.as_str())
            {
                if req.get(&f.json).is_some() {
                    return Err(Fault::invalid("tenant stamp is server-owned"));
                }
                continue;
            }
            if !f.is_column() || f.immutable || f.erased || f.name == e.key {
                continue;
            }
            let text = if f.version {
                let force = req[format!("{}Force", f.json)].as_bool().unwrap_or(false);
                let token = req.get(&f.json);
                if force == token.is_some() {
                    return Err(Fault::invalid("provide one version or force"));
                }
                if let Some(t) = token
                    && value::bind(f, t)?.as_text() != value::bind(f, &row[&f.json])?.as_text()
                {
                    return Err(Fault::aborted("row version changed"));
                }
                Some(q.param(
                    value::bind(f, &value::next_stamp(&row[&f.json], (self.inputs.clock)())?)?,
                    self.db.dialect(),
                ))
            } else if f.nullable && req[format!("{}Null", f.json)] == true {
                Some("NULL".into())
            } else if let Some(v) = req.get(&f.json) {
                let b = self.resolved(f, v)?;
                Some(q.param(b, self.db.dialect()))
            } else {
                None
            };
            if let Some(expr) = text {
                sets.push(format!("{}={expr}", quote(&f.column())))
            }
        }
        let key = e.key().map_err(Fault::invalid)?;
        let table = quote(&e.table());
        let mut updated = row.clone();
        if !sets.is_empty() {
            let mut conditions = self.wall_at(e, &mut q, &table)?;
            conditions.push(format!(
                "{table}.{}={}",
                quote(&key.column()),
                q.param(value::bind(key, &row[&key.json])?, self.db.dialect())
            ));
            let projection = self.project_at(e, &table)?;
            let predicates = self.wall_at(e, &mut q, &table)?;
            let visible = if predicates.is_empty() {
                projection
            } else {
                format!(
                    "CASE WHEN {} THEN {projection} ELSE 'null' END",
                    predicates.join(" AND ")
                )
            };
            q.sql = format!(
                "UPDATE {table} SET {} WHERE {} RETURNING {visible} AS value",
                sets.join(","),
                conditions.join(" AND ")
            );
            let raw = self
                .db
                .rows(q)?
                .into_iter()
                .next()
                .ok_or_else(Fault::missing)?;
            if raw == "null" {
                return Err(Fault::missing());
            }
            updated = self.decode_row(e, &raw)?;
        }
        if !sets.is_empty() {
            self.forget_references();
        }
        let edges_changed = self.write_edges(e, &row[&key.json], req, false)?;
        if !sets.is_empty() || edges_changed {
            self.record(e, method, &req, Some(&row), Some(&updated))?;
        }
        self.image(e, updated, &Value::Null)
    }
    pub(crate) fn erase(&mut self, e: &Entity, reference: &Value) -> Result<Value> {
        let row = match self.lookup(e, reference, true) {
            Ok(row) => row,
            Err(e) if e.code == "not_found" => return Ok(json!({"erased":false})),
            Err(e) => return Err(e),
        };
        let mut q = Query::new("");
        let mut conditions = self.wall(e, &mut q)?;
        let key = e.key().map_err(Fault::invalid)?;
        conditions.push(format!(
            "r.{}={}",
            quote(&key.column()),
            q.param(value::bind(key, &row[&key.json])?, self.db.dialect())
        ));
        let table = quote(&e.table());
        if e.erased().is_none() {
            self.detach_links(e, &row[&key.json])?;
        }
        let mut post = None;
        q.sql = if let Some(stamp) = e.erased() {
            let at = self.inputs.stamp();
            let p = q.param(value::bind(stamp, &at)?, self.db.dialect());
            let mut image = row.clone();
            image[&stamp.json] = at;
            post = Some(image);
            format!(
                "UPDATE {table} AS r SET {}={p} WHERE {}",
                quote(&stamp.column()),
                conditions.join(" AND ")
            )
        } else {
            format!(
                "DELETE FROM {table} AS r WHERE {}",
                conditions.join(" AND ")
            )
        };
        let erased = self.db.exec(q)? == 1;
        if erased {
            self.forget_references();
            self.record(e, "Erase", &Value::Null, Some(&row), post.as_ref())?;
        }
        Ok(json!({"erased":erased}))
    }
    pub(crate) fn dispatch(&mut self, e: &Entity, method: &str, req: &Value) -> Result<Value> {
        match method {
            "$entitlement" => serde_json::to_value(self.entitlement(
                req["key"].as_str().unwrap_or(""),
                req["lock"] == true,
                (self.inputs.clock)(),
            )?)
            .map_err(|e| Fault::internal(e.to_string())),
            "$resource_visible" => self.visible_resource(e, req),
            "$expire_past" => self.expire_history(req),
            "$begin_event" => self.begin_event(e, req),
            "$event_facts" => self.event_facts(e, &req["event"], req),
            "$redact_reason" => self.redact_reason(req),
            "$event" => self.say(e, req),
            "$state" => self.past_state(e, req),
            "Add" => self.insert(e, req),
            "AddMany" => {
                let items = req["items"]
                    .as_array()
                    .ok_or_else(|| Fault::invalid("expected items"))?;
                if items.len() > 1000 {
                    return Err(Fault::invalid("batch size exceeds 1000"));
                }
                let rows = self.insert_items(e, items, true)?;
                Ok(json!({"items":rows}))
            }
            "$watch_start"
                if self
                    .policies
                    .get(&e.name)
                    .is_some_and(|p| p.entity.get("watch").is_some()) =>
            {
                self.watch_start(e, req)
            }
            "$watch_poll"
                if self
                    .policies
                    .get(&e.name)
                    .is_some_and(|p| p.entity.get("watch").is_some()) =>
            {
                self.watch_poll(e, req)
            }
            "List" => self.list(e, req),
            "Get" => {
                let row = self.lookup(e, &req["ref"], false)?;
                self.image(e, row, &req["select"])
            }
            "Patch" => self.patch(e, req, "Patch"),
            "Apply" => self.apply(e, req),
            "Erase" => self.erase(e, req),
            _ => Err(Fault::unsupported(format!("{}.{}", e.name, method))),
        }
    }
}
impl Bind {
    fn as_text(&self) -> Option<&str> {
        if let Self::Text(v) = self {
            Some(v)
        } else {
            None
        }
    }
}
