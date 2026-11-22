use crate::{Fault, Result, driver::Bind};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use chrono::{DateTime, SecondsFormat, Utc};
use paydirt_model::{Field, Scalar};
use serde_json::{Value, json};
pub fn bytes(v: &Value) -> Result<Vec<u8>> {
    let s = v
        .as_str()
        .ok_or_else(|| Fault::invalid("expected base64"))?;
    [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD]
        .iter()
        .find_map(|e| e.decode(s).ok())
        .ok_or_else(|| Fault::invalid("invalid base64"))
}
pub fn encoded(v: &[u8]) -> Value {
    json!(STANDARD.encode(v))
}
pub fn stamp() -> Value {
    json!(Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true))
}
pub fn next_stamp(old: &Value) -> Result<Value> {
    let time = DateTime::parse_from_rfc3339(old.as_str().unwrap_or(""))
        .map_err(|_| Fault::internal("bad stored timestamp"))?
        .to_utc();
    Ok(json!(
        Utc::now()
            .max(time + chrono::TimeDelta::nanoseconds(1))
            .to_rfc3339_opts(SecondsFormat::Nanos, true)
    ))
}
pub fn bind(f: &Field, v: &Value) -> Result<Bind> {
    let bad = || Fault::invalid(format!("invalid value for {}", f.name));
    if !f.collection.is_empty() || matches!(f.kind.as_str(), "JSON" | "MESSAGE") {
        return Ok(Bind::Text(v.to_string()));
    }
    Ok(match f.scalar().map_err(Fault::invalid)? {
        Scalar::Binary => {
            let b = bytes(v)?;
            if f.kind == "UUID" && b.len() != 16 {
                return Err(bad());
            }
            Bind::Bytes(b)
        }
        Scalar::Bool => Bind::Bool(v.as_bool().ok_or_else(bad)?),
        Scalar::Integer => {
            Bind::Integer(i32::try_from(v.as_i64().ok_or_else(bad)?).map_err(|_| bad())?)
        }
        Scalar::BigInt => {
            let n = v
                .as_i64()
                .or_else(|| v.as_str()?.parse().ok())
                .ok_or_else(bad)?;
            if matches!(f.kind.as_str(), "UINT32" | "FIXED32") && u32::try_from(n).is_err() {
                return Err(bad());
            }
            Bind::Big(n)
        }
        Scalar::Double => Bind::Float(v.as_f64().ok_or_else(bad)?),
        Scalar::Text => Bind::Text(match f.kind.as_str() {
            "UINT64" | "FIXED64" => format!(
                "{:020}",
                v.as_u64()
                    .or_else(|| v.as_str()?.parse().ok())
                    .ok_or_else(bad)?
            ),
            "TIME" => DateTime::parse_from_rfc3339(v.as_str().ok_or_else(bad)?)
                .map_err(|_| bad())?
                .to_utc()
                .to_rfc3339_opts(SecondsFormat::Nanos, true),
            "ENUM" => v
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string()),
            _ => v.as_str().ok_or_else(bad)?.to_owned(),
        }),
    })
}
pub fn decode(f: &Field, text: &str) -> Result<Value> {
    let bad = || Fault::internal(format!("corrupt stored {}", f.name));
    if !f.collection.is_empty() || matches!(f.kind.as_str(), "JSON" | "MESSAGE") {
        return serde_json::from_str(text).map_err(|_| bad());
    }
    Ok(match f.kind.as_str() {
        "UUID" | "BYTES" => encoded(&hex::decode(text).map_err(|_| bad())?),
        "BOOL" => json!(matches!(text, "true" | "t" | "1")),
        "INT32" | "SINT32" | "SFIXED32" | "UINT32" | "FIXED32" => {
            json!(text.parse::<i64>().map_err(|_| bad())?)
        }
        "UINT64" | "FIXED64" => json!(text.parse::<u64>().map_err(|_| bad())?.to_string()),
        "FLOAT" | "DOUBLE" => json!(text.parse::<f64>().map_err(|_| bad())?),
        "ENUM" => text
            .parse::<i32>()
            .map(|n| json!(n))
            .unwrap_or_else(|_| json!(text)),
        _ => json!(text),
    })
}
pub fn default_value(f: &Field) -> Value {
    if f.collection == "list" {
        return json!([]);
    }
    if f.collection == "map" {
        return json!({});
    }
    match f.kind.as_str() {
        "TIME" => stamp(),
        "BYTES" | "STRING" => json!(""),
        "BOOL" => json!(false),
        "INT64" | "SINT64" | "SFIXED64" | "UINT64" | "FIXED64" => json!("0"),
        "MESSAGE" | "JSON" => json!({}),
        _ => json!(0),
    }
}
