//! Canonical fact values use field numbers and enum numbers, so renaming a protobuf field is harmless.
use crate::{Fault, Result, value};
use buffa_reflect::{FieldDescriptor, Kind, MessageDescriptor};
use paydirt_model::Field;
use serde_json::{Value, json};

fn scalar(kind: Kind, value: &Value, writing: bool) -> Result<Value> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    match kind {
        Kind::Message(m) if m.full_name() == "google.protobuf.Timestamp" => {
            let at = super::past::time(value)?;
            Ok(json!(
                at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
            ))
        }
        Kind::Message(m) if !m.full_name().starts_with("google.protobuf.") => {
            let mut out = serde_json::Map::new();
            for (name, v) in value
                .as_object()
                .ok_or_else(|| Fault::invalid("expected a message fact"))?
            {
                let field = if writing {
                    m.get_field_by_json_name(name)
                        .or_else(|| m.get_field_by_name(name))
                } else {
                    name.parse().ok().and_then(|n| m.get_field_by_number(n))
                };
                if let Some(f) = field {
                    let key = if writing {
                        f.number().to_string()
                    } else {
                        f.json_name().to_owned()
                    };
                    out.insert(key, reflected(&f, v, writing)?);
                }
            }
            Ok(out.into())
        }
        Kind::Enum(e) if writing && value.is_string() => {
            let v = e
                .get_value_by_name(value.as_str().unwrap())
                .ok_or_else(|| Fault::invalid("unknown enum fact"))?;
            Ok(json!(v.number()))
        }
        Kind::Enum(e) if !writing => Ok(value
            .as_i64()
            .and_then(|n| e.get_value_by_number(n as i32))
            .map(|e| json!(e.name()))
            .unwrap_or_else(|| value.clone())),
        Kind::Int64 | Kind::Uint64 | Kind::Sint64 | Kind::Sfixed64 | Kind::Fixed64 => {
            if writing && let Some(s) = value.as_str() {
                return s
                    .parse::<serde_json::Number>()
                    .map(Value::Number)
                    .map_err(|_| Fault::invalid("invalid integer fact"));
            }
            Ok(if !writing {
                json!(value.to_string())
            } else {
                value.clone()
            })
        }
        _ => Ok(value.clone()),
    }
}
fn reflected(f: &FieldDescriptor, value: &Value, writing: bool) -> Result<Value> {
    if f.is_map()
        && let Kind::Message(entry) = f.kind()
    {
        let item = entry
            .get_field_by_number(2)
            .ok_or_else(|| Fault::internal("map value descriptor missing"))?;
        let fields = value
            .as_object()
            .ok_or_else(|| Fault::invalid("expected a map fact"))?;
        return fields
            .iter()
            .map(|(key, item_value)| Ok((key.clone(), scalar(item.kind(), item_value, writing)?)))
            .collect::<Result<serde_json::Map<_, _>>>()
            .map(Value::Object);
    }
    if f.is_list() {
        return value
            .as_array()
            .ok_or_else(|| Fault::invalid("expected a repeated fact"))?
            .iter()
            .map(|v| scalar(f.kind(), v, writing))
            .collect::<Result<Vec<_>>>()
            .map(Value::Array);
    }
    scalar(f.kind(), value, writing)
}
pub(crate) fn translate(
    desc: Option<&MessageDescriptor>,
    f: &Field,
    v: &Value,
    writing: bool,
) -> Result<Value> {
    if v.is_null() {
        return Ok(Value::Null);
    }
    if !f.target.is_empty() || f.kind == "UUID" {
        if writing {
            let id = value::bytes(v.get("id").unwrap_or(v))?;
            return Ok(json!(
                paydirt::format_id(&id).ok_or_else(|| Fault::invalid("invalid fact UUID"))?
            ));
        }
        let id = v
            .as_str()
            .and_then(paydirt::parse_id)
            .ok_or_else(|| Fault::invalid("invalid stored fact UUID"))?;
        return Ok(if f.target.is_empty() {
            value::encoded(&id)
        } else {
            json!({"id":value::encoded(&id)})
        });
    }
    if let Some(fd) = desc.and_then(|d| d.get_field_by_number(f.number)) {
        if fd.is_map()
            && let Kind::Message(entry) = fd.kind()
        {
            return scalar(
                entry
                    .get_field_by_number(2)
                    .ok_or_else(|| Fault::internal("invalid map entry"))?
                    .kind(),
                v,
                writing,
            );
        }
        return reflected(&fd, v, writing);
    }
    if matches!(
        f.kind.as_str(),
        "INT64" | "SINT64" | "SFIXED64" | "UINT64" | "FIXED64"
    ) {
        return scalar(Kind::Int64, v, writing);
    }
    Ok(v.clone())
}
