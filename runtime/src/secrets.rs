//! Remove schema-declared secret fields from a protobuf payload, including
//! repeated values, map entries and known messages packed in Any.
use crate::{Fault, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
struct Field {
    number: u32,
    #[serde(default)]
    target: String,
    #[serde(default)]
    secret: bool,
}
#[derive(Deserialize)]
struct Message {
    fields: Vec<Field>,
}
pub struct Schema {
    messages: BTreeMap<String, Message>,
    sensitive: BTreeSet<String>,
}
impl Schema {
    pub fn new(descriptor: &str) -> Result<Self> {
        let messages: BTreeMap<String, Message> =
            serde_json::from_str(descriptor).map_err(|e| Fault::internal(e.to_string()))?;
        let mut sensitive = BTreeSet::from(["google.protobuf.Any".into()]);
        loop {
            let n = sensitive.len();
            for (name, m) in &messages {
                if m.fields
                    .iter()
                    .any(|f| f.secret || sensitive.contains(&f.target))
                {
                    sensitive.insert(name.clone());
                }
            }
            if n == sensitive.len() {
                break;
            }
        }
        Ok(Self {
            messages,
            sensitive,
        })
    }
    pub fn protects(&self, name: &str) -> bool {
        self.sensitive.contains(name)
    }
    pub fn clean(&self, ty: &str, wire: &[u8]) -> Result<Vec<u8>> {
        self.clean_at(ty, wire, 0)
    }
    fn clean_at(&self, ty: &str, wire: &[u8], depth: usize) -> Result<Vec<u8>> {
        if depth > 100 {
            return Err(Fault::exhausted("response is too deeply nested"));
        }
        if !self.protects(ty) {
            return Ok(wire.to_vec());
        }
        let mut parts = Vec::new();
        let mut bytes = wire;
        while !bytes.is_empty() {
            let start = bytes;
            let tag = varint(&mut bytes)?;
            let key = &start[..start.len() - bytes.len()];
            let n = usize::try_from(match tag & 7 {
                0 => {
                    varint(&mut bytes)?;
                    0
                }
                1 => 8,
                2 => varint(&mut bytes)?,
                5 => 4,
                _ => return Err(Fault::internal("unsupported response wire type")),
            })
            .map_err(|_| Fault::internal("invalid response length"))?;
            let (payload, rest) = bytes
                .split_at_checked(n)
                .ok_or_else(|| Fault::internal("truncated response"))?;
            let field = u32::try_from(tag >> 3)
                .ok()
                .filter(|v| (1..=536870911).contains(v))
                .ok_or_else(|| Fault::internal("invalid response field"))?;
            parts.push((
                field,
                tag & 7,
                key,
                payload,
                &start[..start.len() - rest.len()],
            ));
            bytes = rest;
        }
        let any = ty == "google.protobuf.Any";
        let packed = if any {
            parts
                .iter()
                .rev()
                .find(|p| p.0 == 1 && p.1 == 2)
                .and_then(|p| std::str::from_utf8(p.3).ok())
                .and_then(|url| url.rsplit('/').next())
        } else {
            None
        };
        let mut out = Vec::new();
        for (number, kind, key, payload, raw) in parts {
            let child = if any {
                match number {
                    1 => None,
                    2 => packed,
                    _ => continue,
                }
            } else {
                let Some(f) = self
                    .messages
                    .get(ty)
                    .and_then(|m| m.fields.iter().find(|f| f.number == number))
                else {
                    continue;
                };
                if f.secret {
                    continue;
                }
                Some(f.target.as_str())
            };
            if kind == 2
                && let Some(child) = child.filter(|t| self.protects(t))
            {
                let clean = self.clean_at(child, payload, depth + 1)?;
                out.extend_from_slice(key);
                put_size(clean.len(), &mut out);
                out.extend(clean);
            } else {
                out.extend_from_slice(raw);
            }
        }
        Ok(out)
    }
}
fn varint(bytes: &mut &[u8]) -> Result<u64> {
    let mut value = 0;
    for i in 0..10 {
        let (&b, rest) = bytes
            .split_first()
            .ok_or_else(|| Fault::internal("truncated response integer"))?;
        *bytes = rest;
        if i == 9 && b > 1 {
            break;
        }
        value |= u64::from(b & 127) << (7 * i);
        if b < 128 {
            return Ok(value);
        }
    }
    Err(Fault::internal("invalid response integer"))
}
fn put_size(mut n: usize, out: &mut Vec<u8>) {
    while n >= 128 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
