use crate::{Fault, Result, Scope, Transaction, value};
use paydirt_model::Entity;
use serde_json::Value;

impl Transaction<'_> {
    fn at(&mut self, e: &Entity, row: &Value, path: &str) -> Result<Value> {
        let (name, rest) = path.split_once('.').unwrap_or((path, ""));
        let field = e.field(name).map_err(Fault::invalid)?;
        let v = row
            .get(&field.json)
            .filter(|v| !v.is_null())
            .ok_or_else(|| Fault::invalid(format!("{name} required for tenant path")))?;
        if field.target.is_empty() {
            return Ok(v.clone());
        }
        let target = self.model.entity(&field.target).map_err(Fault::invalid)?;
        let next = self.reference_row(target, v)?;
        if rest.is_empty() {
            return Ok(next[&target.key().map_err(Fault::internal)?.json].clone());
        }
        self.at(target, &next, rest)
    }

    /// The stamp and agreement checks run on the internal stack as well as the public stack.
    pub(crate) fn admit(&mut self, e: &Entity, req: &Value) -> Result<Value> {
        let Some(policy) = self.policies.get(&e.name) else {
            return Ok(req.clone());
        };
        let p = &policy.entity;
        let t = &p["tenanted"];
        let mut row = req.clone();
        if let Some(fields) = t["field"].as_array() {
            if let Some(ids) = self.scope.tenants() {
                let visible = fields
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(|n| e.field(n).ok())
                    .filter_map(|f| row.get(&f.json))
                    .filter_map(|v| value::bytes(v).ok())
                    .any(|v| ids.contains(&v));
                if !visible {
                    return Err(Fault::missing());
                }
            }
        } else if let Some(path) = t["via"].as_str() {
            let owner = self.at(e, &row, path)?;
            if let Some(stamp) = t["stamp"].as_str() {
                let field = e.field(stamp).map_err(Fault::invalid)?;
                row[&field.json] = owner.clone();
            }
            for path in t["agrees"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let first = path.split('.').next().unwrap();
                if row
                    .get(&e.field(first).map_err(Fault::invalid)?.json)
                    .is_some_and(|v| !v.is_null())
                    && self.at(e, &row, path)? != owner
                {
                    return Err(Fault::invalid(format!("{first} belongs to another tenant")));
                }
            }
        }
        if self.frame.is_some() {
            for (name, options) in &policy.fields {
                let field = e.field(name).map_err(Fault::internal)?;
                if options["stamped"] == true && req.get(&field.json).is_some_and(|v| !v.is_null())
                {
                    return Err(Fault::invalid(format!(
                        "{name} is established by this deployment"
                    )));
                }
            }
        }
        Ok(row)
    }
    pub(crate) fn object_tenant(&mut self, e: &Entity, row: &Value) -> Result<Value> {
        let p = &self.policies[&e.name].entity;
        if p.get("tenant").is_some() {
            return Ok(row[&e.key().map_err(Fault::internal)?.json].clone());
        }
        let t = &p["tenanted"];
        if let Some(name) = t["stamp"].as_str().or_else(|| t["field"].get(0)?.as_str()) {
            return Ok(row[&e.field(name).map_err(Fault::internal)?.json].clone());
        }
        if let Some(path) = t["via"].as_str() {
            return self.at(e, row, path);
        }
        if p.get("global").is_some() {
            return Ok(value::encoded(&[0; 16]));
        }
        let f = e
            .fields
            .iter()
            .find(|f| f.number == 2)
            .ok_or_else(|| Fault::internal("entity has no tenant field"))?;
        Ok(row[&f.json].get("id").unwrap_or(&row[&f.json]).clone())
    }
}
impl Scope {
    pub(crate) fn tenants(&self) -> Option<&[Vec<u8>]> {
        match self {
            Self::Internal => None,
            Self::Tenant(id) => Some(std::slice::from_ref(id)),
            Self::Granted { tenants, .. } => Some(tenants),
        }
    }
}
