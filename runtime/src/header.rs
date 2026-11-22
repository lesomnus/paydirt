//! Schema-declared identity and display fields shared by otherwise unrelated rows.
use crate::{Fault, Result, value};
use chrono::{DateTime, Utc};
use paydirt_model::{Entity, Model};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum Field {
    Id,
    Alias,
    Name,
    Desc,
    Labels,
    Tenant,
    Created,
    Updated,
    Erased,
}
#[derive(Default, Debug, PartialEq)]
pub struct Header {
    pub id: [u8; 16],
    pub alias: String,
    pub name: String,
    pub desc: String,
    pub labels: BTreeMap<String, String>,
    pub tenant: [u8; 16],
    pub created: Option<DateTime<Utc>>,
    pub updated: Option<DateTime<Utc>>,
    pub erased: Option<DateTime<Utc>>,
    fields: BTreeSet<Field>,
}
fn id(v: &Value) -> [u8; 16] {
    value::bytes(v)
        .ok()
        .and_then(|b| b.try_into().ok())
        .unwrap_or_default()
}

impl Header {
    pub fn has(&self, field: Field) -> bool {
        self.fields.contains(&field)
    }
    pub fn title(&self) -> String {
        if !self.name.is_empty() {
            self.name.clone()
        } else if !self.alias.is_empty() {
            self.alias.clone()
        } else if self.id != [0; 16] {
            paydirt::format_id(&self.id).unwrap_or_default()
        } else {
            String::new()
        }
    }
    pub fn read(model: &Model, entity: &Entity, row: &impl Serialize) -> Result<Self> {
        let row = serde_json::to_value(row).map_err(|e| Fault::internal(e.to_string()))?;
        let mut h = Self::default();
        if row.is_null() {
            return Ok(h);
        }
        for f in &entity.fields {
            let v = &row[&f.json];
            let binary = matches!(f.kind.as_str(), "BYTES" | "UUID");
            let part = match f.name.as_str() {
                "id" if binary => {
                    h.id = id(v);
                    Field::Id
                }
                "tenant" | "tenant_id" if !f.target.is_empty() => {
                    let target = model.entity(&f.target).map_err(Fault::internal)?;
                    let Ok(key) = target.field("id") else {
                        continue;
                    };
                    if !matches!(key.kind.as_str(), "BYTES" | "UUID") || v.is_null() {
                        continue;
                    }
                    h.tenant = id(&v[&key.json]);
                    Field::Tenant
                }
                "tenant" | "tenant_id" if binary => {
                    h.tenant = id(v);
                    Field::Tenant
                }
                "alias" | "name" | "desc" if f.kind == "STRING" && f.collection.is_empty() => {
                    let (field, part) = match f.name.as_str() {
                        "alias" => (&mut h.alias, Field::Alias),
                        "name" => (&mut h.name, Field::Name),
                        _ => (&mut h.desc, Field::Desc),
                    };
                    *field = v.as_str().unwrap_or_default().to_owned();
                    part
                }
                "labels" if f.collection == "map" => {
                    if let Some(map) = v.as_object() {
                        h.labels = map
                            .iter()
                            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                            .collect();
                    }
                    Field::Labels
                }
                "date_created" | "date_updated" | "date_erased" if f.kind == "TIME" => {
                    let (field, part) = match f.name.as_str() {
                        "date_created" => (&mut h.created, Field::Created),
                        "date_updated" => (&mut h.updated, Field::Updated),
                        _ => (&mut h.erased, Field::Erased),
                    };
                    *field = v
                        .as_str()
                        .map(str::parse)
                        .transpose()
                        .map_err(|_| Fault::invalid("invalid header timestamp"))?;
                    part
                }
                _ => continue,
            };
            h.fields.insert(part);
        }
        Ok(h)
    }
}
