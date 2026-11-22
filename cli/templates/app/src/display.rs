//! Rendering uses the protobuf field declarations for field order, byte types and table columns.
use super::*;
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
pub(super) struct Ordered<'a> {
    pub value: &'a Value,
    pub ty: &'a str,
    pub schema: &'a Value,
}
impl Serialize for Ordered<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        match self.value {
            Value::Array(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for v in a {
                    seq.serialize_element(&Ordered { value: v, ..*self })?;
                }
                seq.end()
            }
            Value::Object(o) => {
                let mut fields: Vec<_> = o
                    .iter()
                    .map(|(name, v)| {
                        let f = self.schema[self.ty]["fields"].as_array().and_then(|fs| {
                            fs.iter().find(|f| {
                                if self.schema[self.ty]["map"] == true {
                                    f["number"] == 2
                                } else {
                                    f["name"] == name.as_str()
                                }
                            })
                        });
                        (
                            f.and_then(|f| f["number"].as_u64()).unwrap_or(u64::MAX),
                            name,
                            v,
                            f.and_then(|f| f["target"].as_str()).unwrap_or_default(),
                        )
                    })
                    .collect();
                fields.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
                let mut map = s.serialize_map(Some(fields.len()))?;
                for (_, name, v, ty) in fields {
                    map.serialize_entry(
                        name,
                        &Ordered {
                            value: v,
                            ty,
                            schema: self.schema,
                        },
                    )?;
                }
                map.end()
            }
            other => other.serialize(s),
        }
    }
}
pub(super) fn pretty(v: &mut Value, ty: &str, schema: &Value) {
    if let Some(a) = v.as_array_mut() {
        for item in a {
            pretty(item, ty, schema);
        }
        return;
    }
    let Some(o) = v.as_object_mut() else { return };
    let Some(fields) = schema[ty]["fields"].as_array() else {
        return;
    };
    for f in fields {
        let Some(name) = f["name"].as_str() else {
            continue;
        };
        let Some(value) = o.get_mut(name) else {
            continue;
        };
        if f["bytes"] == true && f["uuid"] != true {
            let convert = |v: &mut Value| {
                if let Some(b) = v.as_str().and_then(|s| STANDARD.decode(s).ok()) {
                    *v = json!(format!("({} bytes)", b.len()));
                }
            };
            if let Some(a) = value.as_array_mut() {
                a.iter_mut().for_each(convert)
            } else {
                convert(value)
            }
        } else if f["target"] == "google.protobuf.Timestamp"
            && value
                .as_str()
                .is_some_and(|s| s.starts_with("0001-01-01T00:00:00"))
        {
            o.remove(name);
        } else if let Some(target) = f["target"].as_str() {
            pretty(value, target, schema);
        }
    }
}
fn cell(v: &Value) -> String {
    match v {
        Value::Null => "-".into(),
        Value::String(s) if s.is_empty() => "-".into(),
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}
fn age(v: &Value) -> String {
    let Some(at) = v
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
    else {
        return "-".into();
    };
    let secs = (chrono::Utc::now() - at.to_utc()).num_seconds().max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}
fn path_value<'a, 's>(
    mut row: &'a Value,
    mut ty: &'s str,
    path: &str,
    schema: &'s Value,
) -> &'a Value {
    for part in path.split('.') {
        let f = schema[ty]["fields"]
            .as_array()
            .and_then(|fs| fs.iter().find(|f| f["name"] == part || f["proto"] == part));
        row = &row[f.and_then(|f| f["name"].as_str()).unwrap_or(part)];
        ty = f.and_then(|f| f["target"].as_str()).unwrap_or("");
    }
    row
}
pub(super) fn table(
    v: &Value,
    entity: &str,
    schema: &Value,
    declared: &Value,
    tree: &crate::tree::Tree,
    wide: bool,
    watch: bool,
    header: &mut bool,
) -> Result<()> {
    let custom = tree.columns.get(entity);
    let mut columns: Vec<(String, String)> = if let Some(cols) = custom {
        cols.iter()
            .map(|(title, path)| (title.to_string(), path.to_string()))
            .collect()
    } else if let Some(cols) = declared.as_array().filter(|c| !c.is_empty()) {
        cols.iter()
            .filter(|f| wide || f["wide"] != true)
            .filter_map(|f| {
                let path = f["field"].as_str()?;
                Some((
                    f["title"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| path.to_uppercase()),
                    path.to_owned(),
                ))
            })
            .collect()
    } else {
        let mut cols = vec![
            ("ALIAS".into(), "alias".into()),
            ("AGE".into(), "$age".into()),
        ];
        if wide || watch {
            cols.push(("Id".into(), "id".into()));
        }
        cols
    };
    if watch {
        columns.insert(0, ("ACTION".into(), "$action".into()));
    }
    let mut out = io::stdout().lock();
    if !*header {
        writeln!(
            out,
            "{}",
            columns
                .iter()
                .map(|(t, _)| t.as_str())
                .collect::<Vec<_>>()
                .join("\t")
        )?;
        *header = true;
    }
    let rows = v["items"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(v));
    for item in rows {
        let row = if watch { &item["value"] } else { item };
        let cells = columns
            .iter()
            .map(|(_, path)| match path.as_str() {
                "$action" => item["action"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Snapshot")
                    .into(),
                "$age" => age(&row["dateCreated"]),
                "id" if watch && row["id"].is_null() => item["id"]
                    .as_str()
                    .and_then(|s| STANDARD.decode(s).ok())
                    .and_then(|v| paydirt::format_id(&v))
                    .unwrap_or_else(|| cell(&item["id"])),
                p if p.starts_with('/') => cell(row.pointer(p).unwrap_or(&Value::Null)),
                p => cell(path_value(row, entity, p, schema)),
            })
            .collect::<Vec<_>>();
        writeln!(out, "{}", cells.join("\t"))?;
    }
    if let Some(next) = v["next"].as_str().filter(|s| !s.is_empty()) {
        writeln!(out, "next\t{next}")?;
    }
    out.flush()?;
    Ok(())
}
