use crate::{
    Fault, Result, Transaction,
    driver::{Bind, Query},
    value,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use paydirt_model::{Dialect, Entity, Field, quote};
use serde_json::{Value, json};

fn from_cursor(field: &Field, v: &Value) -> Result<Value> {
    if field.kind == "UUID" {
        let uuid = v
            .as_str()
            .ok_or_else(|| Fault::invalid("invalid UUID cursor"))?;
        let bytes = hex::decode(uuid.replace('-', ""))
            .map_err(|_| Fault::invalid("invalid UUID cursor"))?;
        if bytes.len() != 16 {
            return Err(Fault::invalid("invalid UUID cursor"));
        }
        return Ok(value::encoded(&bytes));
    }
    Ok(v.clone())
}
fn to_cursor(field: &Field, v: &Value) -> Result<Value> {
    if field.kind == "UUID" {
        let hex = hex::encode(value::bytes(v)?);
        if hex.len() != 32 {
            return Err(Fault::internal("invalid stored UUID"));
        }
        return Ok(json!(format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        )));
    }
    Ok(v.clone())
}

impl Transaction<'_> {
    fn same(&mut self, q: &mut Query, f: &Field, v: &Value) -> Result<String> {
        let bound = match self.resolved(f, v) {
            Ok(b) => b,
            Err(err) if err.code == "not_found" => return Ok("1=0".into()),
            Err(err) => return Err(err),
        };
        let param = q.param(bound, self.db.dialect());
        Ok(format!("r.{}={param}", quote(&f.column())))
    }
    fn filters(&mut self, e: &Entity, q: &mut Query, fs: &[Value], spec: &Value) -> Result<String> {
        if fs.len() > 32 {
            return Err(Fault::invalid("at most 32 filters"));
        }
        let mut any = Vec::new();
        for filter in fs {
            let mut and = Vec::new();
            for (name, v) in filter
                .as_object()
                .ok_or_else(|| Fault::invalid("filter must be a message"))?
            {
                if name == "ref" {
                    for (f, part) in self.key_parts(e, v)? {
                        and.push(self.same(q, f, part)?);
                    }
                } else {
                    let name = spec["by"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v["name"].as_str())
                        .find(|field| heck::AsLowerCamelCase(*field).to_string() == *name)
                        .ok_or_else(|| Fault::invalid("undeclared filter"))?;
                    let f = e.field(name).map_err(Fault::invalid)?;
                    if f.collection == "map" {
                        let entries = v
                            .as_object()
                            .ok_or_else(|| Fault::invalid("expected label map"))?;
                        if entries.is_empty() {
                            continue;
                        }
                        if entries.values().any(|value| !value.is_string()) {
                            return Err(Fault::invalid("expected string label values"));
                        }
                        let dialect = self.db.dialect();
                        let wanted = q.param(Bind::Text(v.to_string()), dialect);
                        let column = quote(&f.column());
                        and.push(match dialect {
                            Dialect::Postgres => format!("r.{column}::jsonb @> {wanted}::jsonb"),
                            Dialect::Sqlite => format!(
                                "NOT EXISTS (SELECT 1 FROM json_each({wanted}) want WHERE NOT EXISTS \
                                 (SELECT 1 FROM json_each(r.{column}) have WHERE have.key=want.key \
                                  AND have.type='text' AND have.value=want.value))"
                            ),
                        });
                    } else {
                        and.push(self.same(q, f, v)?);
                    }
                }
            }
            if and.is_empty() {
                return Err(Fault::invalid("filter names nothing"));
            }
            any.push(format!("({})", and.join(" AND ")));
        }
        Ok(format!("({})", any.join(" OR ")))
    }
    pub(crate) fn list(&mut self, e: &Entity, req: &Value) -> Result<Value> {
        let spec = &self
            .policies
            .get(&e.name)
            .ok_or_else(|| Fault::unsupported("no list"))?
            .entity["list"];
        if spec.is_null() {
            return Err(Fault::unsupported("list is not declared"));
        }
        let mut q = Query::new("");
        let mut clauses = self.wall(e, &mut q)?;
        if let Some(fs) = req["filters"].as_array().filter(|fs| !fs.is_empty()) {
            clauses.push(self.filters(e, &mut q, fs, spec)?);
        }
        let order_name = req["order"].as_str().unwrap_or("");
        let prefix = format!(
            "{}_LIST_ORDER_",
            heck::AsShoutySnakeCase(e.name.rsplit('.').next().unwrap())
        );
        let order_number = req["order"].as_i64().unwrap_or(0);
        let mut tag = String::new();
        let mut columns = &spec["order"];
        if order_number != 0
            || !matches!(order_name, "") && order_name != format!("{prefix}UNSPECIFIED")
        {
            let orders = spec["orders"]
                .as_array()
                .ok_or_else(|| Fault::invalid("unknown list order"))?;
            let entry = orders
                .iter()
                .enumerate()
                .find(|(i, o)| {
                    order_number == *i as i64 + 1
                        || o["name"]
                            .as_str()
                            .is_some_and(|n| order_name == format!("{prefix}{}", n.to_uppercase()))
                })
                .ok_or_else(|| Fault::invalid("unknown list order"))?
                .1;
            tag = entry["name"].as_str().unwrap().into();
            columns = &entry["order"];
        }
        let reverse = req["reverse"] == true;
        if reverse {
            tag.push_str("/reverse");
        }
        let order = columns
            .as_array()
            .ok_or_else(|| Fault::invalid("missing list order"))?
            .iter()
            .map(|o| {
                Ok((
                    e.field(
                        o["field"]["name"]
                            .as_str()
                            .ok_or_else(|| Fault::invalid("missing order field"))?,
                    )
                    .map_err(Fault::invalid)?,
                    (o["desc"] == true) ^ reverse,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        if let Some(token) = req["after"].as_str().filter(|s| !s.is_empty()) {
            let bad = || Fault::invalid("cursor does not match list order");
            let bytes = URL_SAFE_NO_PAD.decode(token).map_err(|_| bad())?;
            let values: Vec<Value> = serde_json::from_slice(&bytes).map_err(|_| bad())?;
            let prefix = usize::from(!tag.is_empty());
            if values.len() != order.len() + prefix || (prefix == 1 && values[0] != tag) {
                return Err(bad());
            }
            let mut equals = Vec::new();
            let mut after = Vec::new();
            for ((f, desc), v) in order.iter().zip(&values[prefix..]) {
                let v = from_cursor(f, v)?;
                let b = value::bind(f, &v)?;
                let param = q.param(b, self.db.dialect());
                let col = format!("r.{}", quote(&f.column()));
                let mut leg = equals.clone();
                leg.push(format!("{col}{}{param}", if *desc { "<" } else { ">" }));
                after.push(format!("({})", leg.join(" AND ")));
                equals.push(format!("{col}={param}"));
            }
            clauses.push(format!("({})", after.join(" OR ")));
        }
        let default = spec["size"].as_i64().unwrap_or(50);
        let max = spec["max"].as_i64().unwrap_or(100);
        let n = req["size"]
            .as_i64()
            .filter(|v| *v > 0)
            .unwrap_or(default)
            .min(max);
        let by = order
            .iter()
            .map(|(f, d)| {
                format!(
                    "r.{} {}",
                    quote(&f.column()),
                    if *d { "DESC" } else { "ASC" }
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        if clauses.is_empty() {
            clauses.push("1=1".into());
        }
        q.sql = format!(
            "SELECT {} AS value FROM {} AS r WHERE {} ORDER BY {by} LIMIT {}",
            self.project_sql(e)?,
            quote(&e.table()),
            clauses.join(" AND "),
            n + 1
        );
        let mut rows = self
            .db
            .rows(q)?
            .iter()
            .map(|raw| self.decode_row(e, raw))
            .collect::<Result<Vec<_>>>()?;
        let mut next = String::new();
        if rows.len() > n as usize {
            rows.pop();
            let last = rows
                .last()
                .ok_or_else(|| Fault::invalid("invalid page limit"))?;
            let mut values = Vec::new();
            if !tag.is_empty() {
                values.push(json!(tag));
            }
            for (f, _) in &order {
                values.push(to_cursor(f, &last[&f.json])?);
            }
            next = URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&values).map_err(|e| Fault::internal(e.to_string()))?);
        }
        let mut select = json!({"all":true});
        for field in spec["with"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["name"].as_str())
        {
            let f = e.field(field).map_err(Fault::invalid)?;
            select[&f.json] = json!({"all":true});
        }
        let items = rows
            .into_iter()
            .map(|row| self.image(e, row, &select))
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({"items":items,"next":next}))
    }
}
