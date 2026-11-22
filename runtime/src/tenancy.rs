use crate::{Fault, Result, Scope, Transaction, value};
use paydirt_model::Entity;
use serde_json::Value;

impl Transaction<'_> {
    fn at(&mut self, e: &Entity, row: &Value, path: &str) -> Result<Value> {
        let (name, rest) = path.split_once('.').unwrap_or((path, ""));
        let field = e.field(name).map_err(Fault::invalid)?;
        let Some(v) = row.get(&field.json).filter(|v| !v.is_null()) else {
            if field.nullable {
                return Ok(value::encoded(&[0; 16]));
            }
            return Err(Fault::invalid(format!("{name} required for tenant path")));
        };
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
        self.check_membership_input(e, req)?;
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
        if !matches!(self.scope, Scope::Internal)
            && self.frame.is_some()
            && !self.origin.as_ref().is_some_and(|c| {
                c.entity == e.name && matches!(c.method.as_str(), "Add" | "AddMany")
            })
        {
            self.check_stamps(e, req)?;
        }
        Ok(row)
    }
    pub(crate) fn check_stamps(&self, e: &Entity, item: &Value) -> Result<()> {
        for (name, options) in &self.policies[&e.name].fields {
            let field = e.field(name).map_err(Fault::internal)?;
            if options["stamped"] == true && item.get(&field.json).is_some_and(|v| !v.is_null()) {
                return Err(Fault::field(name, "established by this deployment"));
            }
        }
        Ok(())
    }
    pub(crate) fn check_add_input(
        &mut self,
        e: &Entity,
        request: &Value,
        many: bool,
    ) -> Result<Value> {
        if !matches!(self.scope, Scope::Internal) && self.policies[&e.name].deployment_method("Add")
        {
            return Err(Fault::unsupported("the deployment owns this operation"));
        }
        let single = std::slice::from_ref(request);
        let items = if many {
            request["items"]
                .as_array()
                .ok_or_else(|| Fault::invalid("expected items"))?
                .as_slice()
        } else {
            single
        };
        if items.len() > 1000 {
            return Err(Fault::invalid("batch size exceeds 1000"));
        }
        for (index, item) in items.iter().enumerate() {
            let result = (|| {
                self.admit(e, item)?;
                for f in &e.fields {
                    if f.is_column()
                        && !f.target.is_empty()
                        && let Some(key) = item.get(&f.json).filter(|v| !v.is_null())
                    {
                        let target = self.model.entity(&f.target).map_err(Fault::internal)?;
                        self.reference_row(target, key)?;
                    }
                }
                Ok(())
            })();
            result.map_err(|err| Self::item_error(err, index, many))?;
        }
        Ok(Value::Null)
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
            let mut entity = e;
            let mut image = row.clone();
            let mut steps = path.split('.').peekable();
            while let Some(step) = steps.next() {
                let field = entity.field(step).map_err(Fault::internal)?;
                let v = &image[&field.json];
                if v.is_null() {
                    return Ok(value::encoded(&[0; 16]));
                }
                if field.target.is_empty() {
                    return Ok(v.clone());
                }
                entity = self.model.entity(&field.target).map_err(Fault::internal)?;
                let id = &v[&entity.key().map_err(Fault::internal)?.json];
                if steps.peek().is_none() {
                    return Ok(id.clone());
                }
                image = self
                    .stored_row(entity, id, false)?
                    .ok_or_else(Fault::missing)?;
            }
            return Err(Fault::invalid("empty tenant path"));
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
            Self::Account { .. } => Some(&[]),
            Self::Tenant(id) => Some(std::slice::from_ref(id)),
            Self::Granted { tenants, .. } => Some(tenants),
        }
    }
}
