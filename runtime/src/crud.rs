use crate::{
    Fault, Result, Scope, Transaction,
    driver::{Bind, Query},
    value,
};
use paydirt_model::{Dialect, Entity, Field, Scalar, quote};
use serde_json::{Map, Value, json};
type Object = Map<String, Value>;

impl Transaction<'_> {
    fn field_type<'a>(&'a self, f: &'a Field) -> Result<&'a Field> {
        if f.target.is_empty() {
            return Ok(f);
        }
        self.model
            .entity(&f.target)
            .and_then(Entity::key)
            .map_err(Fault::invalid)
    }
    fn project_sql(&self, e: &Entity) -> Result<String> {
        let d = self.db.dialect();
        let mut cols = Vec::new();
        for f in e.fields.iter().filter(|f| f.is_column()) {
            let c = format!("r.{}", quote(&f.column()));
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
    fn decode_row(&self, e: &Entity, row: &str) -> Result<Value> {
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
    pub(crate) fn path(&self, e: &Entity, path: &str, alias: &str, depth: u32) -> Result<String> {
        let (name, rest) = path.split_once('.').unwrap_or((path, ""));
        let f = e.field(name).map_err(Fault::invalid)?;
        let col = format!("{alias}.{}", quote(&f.column()));
        if rest.is_empty() {
            return Ok(col);
        }
        let target = self.model.entity(&f.target).map_err(Fault::invalid)?;
        let next = format!("s{depth}");
        let nested = self.path(target, rest, &next, depth + 1)?;
        let visible = target
            .erased()
            .map(|f| format!(" AND {next}.{} IS NULL", quote(&f.column())))
            .unwrap_or_default();
        Ok(format!(
            "(SELECT {nested} FROM {} AS {next} WHERE {next}.{}={col}{visible})",
            quote(&target.table()),
            quote(&target.key().map_err(Fault::invalid)?.column())
        ))
    }
    fn wall(&self, e: &Entity, q: &mut Query) -> Result<Vec<String>> {
        let mut parts = Vec::new();
        if let Some(f) = e.erased() {
            parts.push(format!("r.{} IS NULL", quote(&f.column())))
        }
        let Scope::Tenant(id) = self.scope else {
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
        } else if let Some(v) = p["tenanted"]["via"].as_str() {
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
        for path in paths {
            let name = self.path(e, path, "r", 0)?;
            let v = q.param(Bind::Bytes(id.clone()), self.db.dialect());
            any.push(format!("{name}={v}"));
        }
        parts.push(format!("({})", any.join(" OR ")));
        Ok(parts)
    }
    fn resolved(&mut self, f: &Field, input: &Value) -> Result<Bind> {
        if f.target.is_empty() {
            return value::bind(f, input);
        }
        let target = self.model.entity(&f.target).map_err(Fault::invalid)?;
        let row = self.lookup(target, input, false)?;
        let key = target.key().map_err(Fault::invalid)?;
        value::bind(key, &row[&key.json])
    }
    fn key_parts<'a>(&self, e: &'a Entity, key: &'a Value) -> Result<Vec<(&'a Field, &'a Value)>> {
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
    fn lookup(&mut self, e: &Entity, key: &Value, lock: bool) -> Result<Value> {
        let mut q = Query::new("");
        let mut where_ = self.wall(e, &mut q)?;
        for (f, v) in self.key_parts(e, key)? {
            let b = self.resolved(f, v)?;
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
        let mut q = Query::new("");
        let mut cols = Vec::new();
        let mut params = Vec::new();
        let key = e.key().map_err(Fault::invalid)?;
        let mut id = Value::Null;
        for f in e.fields.iter().filter(|f| f.is_column()) {
            let supplied = req.get(&f.json).filter(|v| !v.is_null());
            let v = if f.erased {
                continue;
            } else if f.version {
                value::stamp()
            } else if let Some(v) = supplied {
                v.clone()
            } else if f.name == e.key
                && let Some(domain) = self.domain(e)
            {
                value::encoded(&paydirt::mint(domain))
            } else if f.nullable {
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
                value::default_value(f)
            };
            if f.name == e.key {
                if let Some(domain) = self.domain(e)
                    && !paydirt::has_domain(&value::bytes(&v)?, domain)
                {
                    return Err(Fault::invalid("incorrect Payday ID domain"));
                }
                id = v.clone();
            }
            let b = self.resolved(f, &v)?;
            cols.push(quote(&f.column()));
            params.push(q.param(b, self.db.dialect()));
        }
        q.sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote(&e.table()),
            cols.join(","),
            params.join(",")
        );
        self.db.exec(q)?;
        // The postimage must belong to the caller as well. Failure rolls back the insert.
        let row = self.lookup(e, &json!({&key.json:id}), false)?;
        self.record(e, "Add", None, Some(&row))?;
        self.image(e, row, &Value::Null)
    }
    fn patch(&mut self, e: &Entity, req: &Value) -> Result<Value> {
        let row = self.lookup(e, &req["ref"], true)?;
        let mut q = Query::new("");
        let mut sets = Vec::new();
        for f in &e.fields {
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
                    value::bind(f, &value::next_stamp(&row[&f.json])?)?,
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
        let reference = json!({&key.json:&row[&key.json]});
        if !sets.is_empty() {
            let mut conditions = self.wall(e, &mut q)?;
            conditions.push(format!(
                "r.{}={}",
                quote(&key.column()),
                q.param(value::bind(key, &row[&key.json])?, self.db.dialect())
            ));
            q.sql = format!(
                "UPDATE {} AS r SET {} WHERE {}",
                quote(&e.table()),
                sets.join(","),
                conditions.join(" AND ")
            );
            if self.db.exec(q)? != 1 {
                return Err(Fault::missing());
            }
        }
        let updated = self.lookup(e, &reference, false)?;
        self.record(e, "Patch", Some(&row), Some(&updated))?;
        self.image(e, updated, &Value::Null)
    }
    fn erase(&mut self, e: &Entity, reference: &Value) -> Result<Value> {
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
        q.sql = if let Some(stamp) = e.erased() {
            format!(
                "UPDATE {table} AS r SET {}={} WHERE {}",
                quote(&stamp.column()),
                q.param(value::bind(stamp, &value::stamp())?, self.db.dialect()),
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
            self.record(e, "Erase", Some(&row), None)?;
        }
        Ok(json!({"erased":erased}))
    }
    pub(crate) fn dispatch(&mut self, e: &Entity, method: &str, req: &Value) -> Result<Value> {
        match method {
            "Add" => self.insert(e, req),
            "AddMany" => {
                let items = req["items"]
                    .as_array()
                    .ok_or_else(|| Fault::invalid("expected items"))?;
                if items.len() > 1000 {
                    return Err(Fault::invalid("batch size exceeds 1000"));
                }
                let rows = items
                    .iter()
                    .map(|v| self.insert(e, v))
                    .collect::<Result<Vec<_>>>()?;
                Ok(json!({"items":rows}))
            }
            "Get" => {
                let row = self.lookup(e, &req["ref"], false)?;
                self.image(e, row, &req["select"])
            }
            "Patch" => self.patch(e, req),
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
