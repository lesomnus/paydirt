use crate::{Fault, Result, Transaction, value};
use paydirt_model::{Entity, Field};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn field<'a>(e: &'a Entity, spec: &Value) -> Result<Option<&'a Field>> {
    let mut found = None;
    let parts = spec
        .as_object()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Fault::invalid("missing field selector"))?;
    for (kind, v) in parts {
        let next = e.fields.iter().find(|f| match kind.as_str() {
            "name" => v == &f.name,
            "jsonName" => v == &f.json,
            "number" => v.as_u64() == Some(u64::from(f.number)),
            _ => false,
        });
        if found.is_some_and(|old: Option<&Field>| old.map(|f| f.number) != next.map(|f| f.number))
        {
            return Err(Fault::invalid("patch field identifiers disagree"));
        }
        found = Some(next);
    }
    Ok(found.flatten())
}
fn arm(v: &Value) -> Result<(&str, &Value)> {
    let o = v
        .as_object()
        .filter(|v| v.len() == 1)
        .ok_or_else(|| Fault::invalid("expected one typed patch value"))?;
    let (k, v) = o.iter().next().unwrap();
    Ok((k, v))
}
fn literal(v: &Value) -> Result<Value> {
    let (tag, v) = arm(v)?;
    Ok(match tag {
        "l" => Value::Array(
            v["values"]
                .as_array()
                .into_iter()
                .flatten()
                .map(literal)
                .collect::<Result<_>>()?,
        ),
        "map" => {
            let mut out = json!({});
            for item in v["entries"].as_array().into_iter().flatten() {
                let (_, key) = arm(&item["key"])?;
                let key = key
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| key.to_string());
                out[key] = literal(&item["value"])?;
            }
            out
        }
        "s" | "b" | "i32" | "i64" | "u32" | "u64" | "f32" | "f64" | "x" | "e" => v.clone(),
        _ => {
            return Err(Fault::unsupported(
                "patch literal is not a mapped column type",
            ));
        }
    })
}
fn typed(f: &Field, v: &Value) -> Result<Value> {
    let (tag, content) = arm(v)?;
    if f.kind == "TIME" && tag == "m" {
        let (mut seconds, mut nanos) = (0, 0);
        for item in content["fields"].as_array().into_iter().flatten() {
            let k = &item["key"];
            let (_, v) = arm(&item["value"])?;
            if k["number"] == 1 || k["name"] == "seconds" {
                seconds = v
                    .as_i64()
                    .or_else(|| v.as_str()?.parse().ok())
                    .ok_or_else(|| Fault::invalid("invalid timestamp"))?;
            } else if k["number"] == 2 || k["name"] == "nanos" {
                nanos = v
                    .as_u64()
                    .and_then(|n| n.try_into().ok())
                    .ok_or_else(|| Fault::invalid("invalid nanoseconds"))?;
            } else {
                return Err(Fault::invalid("unknown timestamp field"));
            }
        }
        let t = chrono::DateTime::from_timestamp(seconds, nanos)
            .ok_or_else(|| Fault::invalid("invalid timestamp"))?;
        return Ok(json!(t.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)));
    }
    let expected = if !f.collection.is_empty() {
        if f.collection == "map" { "map" } else { "l" }
    } else {
        match f.kind.as_str() {
            "STRING" => "s",
            "BYTES" | "UUID" => "x",
            "BOOL" => "b",
            "INT32" | "SINT32" | "SFIXED32" => "i32",
            "INT64" | "SINT64" | "SFIXED64" => "i64",
            "UINT32" | "FIXED32" => "u32",
            "UINT64" | "FIXED64" => "u64",
            "FLOAT" => "f32",
            "DOUBLE" => "f64",
            "ENUM" => "e",
            _ => return Err(Fault::unsupported("unsupported column literal")),
        }
    };
    if tag != expected {
        return Err(Fault::invalid(format!(
            "{} expects a {expected} literal",
            f.name
        )));
    }
    let v = literal(v)?;
    value::bind(f, &v)?;
    Ok(v)
}
fn test(
    test: &Value,
    observed: Option<&Value>,
    wanted: impl FnOnce(&Value) -> Result<Value>,
) -> Result<()> {
    let ok = if let Some(exists) = test["exists"].as_bool() {
        observed.is_some() == exists
    } else if let Some(v) = test.get("value") {
        Some(&wanted(v)?) == observed
    } else {
        return Err(Fault::invalid("missing patch test condition"));
    };
    if ok {
        Ok(())
    } else {
        Err(Fault::precondition("a test in the patch did not hold"))
    }
}

struct Plan<'a> {
    row: &'a Value,
    patch: Value,
    written: BTreeSet<u32>,
    deletes: Vec<(String, usize)>,
}
impl Plan<'_> {
    fn entry(
        &mut self,
        tx: &Transaction<'_>,
        e: &Entity,
        entry: &Value,
        inherited: Option<&Field>,
        depth: u8,
    ) -> Result<()> {
        if depth > 32 {
            return Err(Fault::invalid("patch nesting exceeds limit"));
        }
        let acts = ["test", "assign", "remove", "insert", "nest", "move", "copy"];
        let mut acts = acts.iter().copied().filter(|k| entry.get(k).is_some());
        let op = acts
            .next()
            .ok_or_else(|| Fault::invalid("missing patch operation"))?;
        if acts.next().is_some() {
            return Err(Fault::invalid("multiple patch operations"));
        }
        let skip = entry["onMissing"] == "ON_MISSING_SKIP" || entry["onMissing"] == 1;
        if op == "test" && skip {
            return Err(Fault::invalid("patch tests cannot skip missing targets"));
        }
        let mut container = inherited;
        for segment in entry["path"]["segments"].as_array().into_iter().flatten() {
            if container.is_some() {
                return Err(Fault::unsupported("only one mapped column is addressable"));
            }
            container = Some(
                field(e, &segment["field"])?.ok_or_else(|| Fault::invalid("unknown patch path"))?,
            );
        }
        if entry.get("container").is_some() {
            let f = container.ok_or_else(|| Fault::unsupported("cannot replace the entire row"))?;
            if f.collection.is_empty() {
                return Err(Fault::unsupported("container must be a collection"));
            }
            if op == "nest" {
                for entry in entry["nest"]["delta"]["entries"]
                    .as_array()
                    .ok_or_else(|| Fault::invalid("missing nested delta"))?
                {
                    self.entry(tx, e, entry, Some(f), depth + 1)?
                }
                return Ok(());
            }
            self.mutable(e, f, tx)?;
            self.patch[&f.json] = match op {
                "assign" => typed(f, &entry["assign"]["value"])?,
                "remove" => value::default_value(f, tx.inputs),
                "insert" if f.collection == "list" => {
                    let mut out = self.current(f).clone();
                    let v = typed(f, &entry["insert"]["value"])?;
                    out.as_array_mut()
                        .ok_or_else(|| Fault::invalid("expected list"))?
                        .extend(v.as_array().unwrap().iter().cloned());
                    out
                }
                _ => return Err(Fault::unsupported("unsupported container operation")),
            };
            return Ok(());
        }
        let selectors = entry["targets"]["selectors"]
            .as_array()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Fault::invalid("patch operation requires targets"))?;
        for selector in selectors {
            if let Some(f) = container {
                if !f.target.is_empty() {
                    let target = tx.model.entity(&f.target).map_err(Fault::invalid)?;
                    let key = target.key().map_err(Fault::invalid)?;
                    if field(target, &selector["key"]["field"])?.map(|f| f.number)
                        != Some(key.number)
                    {
                        return Err(Fault::unsupported("only the referenced key is stored"));
                    }
                    if op == "test" {
                        self.guard(f, entry, self.row[&f.json].get(&key.json), |v| {
                            typed(key, v)
                        })?;
                    } else if op == "assign" {
                        self.mutable(e, f, tx)?;
                        self.patch[&f.json] = json!({&key.json:typed(key,&entry[op]["value"])?});
                    } else {
                        return Err(Fault::unsupported("unsupported edge key operation"));
                    }
                } else {
                    self.collection(e, f, selector, entry, op, skip, tx)?;
                }
                continue;
            }
            let Some(f) = field(e, &selector["key"]["field"])? else {
                if op == "test" {
                    test(&entry["test"], None, literal)?;
                } else if !skip {
                    return Err(Fault::invalid("patch field does not exist"));
                }
                continue;
            };
            match op {
                "test" => {
                    let key = if f.target.is_empty() {
                        f
                    } else {
                        tx.model
                            .entity(&f.target)
                            .and_then(Entity::key)
                            .map_err(Fault::invalid)?
                    };
                    let observed = self.row.get(&f.json).map(|v| {
                        if f.target.is_empty() {
                            v
                        } else {
                            &v[&key.json]
                        }
                    });
                    if !f.collection.is_empty() && entry["test"].get("value").is_some() {
                        return Err(Fault::unsupported("test individual collection entries"));
                    }
                    self.guard(f, entry, observed, |v| typed(key, v))?;
                }
                "nest" => {
                    for sub in entry["nest"]["delta"]["entries"]
                        .as_array()
                        .ok_or_else(|| Fault::invalid("missing nested delta"))?
                    {
                        self.entry(tx, e, sub, Some(f), depth + 1)?;
                    }
                }
                "assign" => {
                    self.mutable(e, f, tx)?;
                    if !f.target.is_empty() {
                        return Err(Fault::unsupported("address the edge key"));
                    }
                    self.patch[&f.json] = typed(f, &entry[op]["value"])?;
                    self.patch
                        .as_object_mut()
                        .unwrap()
                        .remove(&format!("{}Null", f.json));
                }
                "remove" => {
                    self.mutable(e, f, tx)?;
                    if !f.collection.is_empty() {
                        self.patch[&f.json] = value::default_value(f, tx.inputs);
                    } else if f.nullable {
                        self.patch[format!("{}Null", f.json)] = json!(true);
                    } else {
                        return Err(Fault::unsupported("cannot clear a required column"));
                    }
                }
                _ => {
                    return Err(Fault::unsupported(
                        "operation is not supported on a row field",
                    ));
                }
            }
        }
        Ok(())
    }
    fn mutable(&mut self, e: &Entity, f: &Field, tx: &Transaction<'_>) -> Result<()> {
        if f.immutable
            || f.version
            || f.erased
            || f.name == e.key
            || tx
                .policies
                .get(&e.name)
                .is_some_and(|p| p.entity["tenanted"]["stamp"] == f.name)
        {
            return Err(Fault::invalid(format!(
                "{} is server-owned or immutable",
                f.name
            )));
        }
        self.written.insert(f.number);
        Ok(())
    }
    fn current(&self, f: &Field) -> &Value {
        self.patch.get(&f.json).unwrap_or(&self.row[&f.json])
    }
    fn guard(
        &self,
        f: &Field,
        entry: &Value,
        observed: Option<&Value>,
        want: impl FnOnce(&Value) -> Result<Value>,
    ) -> Result<()> {
        if self.written.contains(&f.number) {
            return Err(Fault::unsupported("patch tests a column after writing it"));
        }
        test(&entry["test"], observed, want)
    }
    fn collection(
        &mut self,
        e: &Entity,
        f: &Field,
        sel: &Value,
        entry: &Value,
        op: &str,
        skip: bool,
        tx: &Transaction<'_>,
    ) -> Result<()> {
        let mut data = self.current(f).clone();
        match f.collection.as_str() {
            "map" => {
                let entries = data
                    .as_object_mut()
                    .ok_or_else(|| Fault::invalid("expected map"))?;
                if sel.get("everyEntry").is_some() && op == "remove" {
                    self.mutable(e, f, tx)?;
                    entries.clear();
                } else {
                    let (_, k) = arm(&sel["key"]["mapKey"])?;
                    let key = k
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| k.to_string());
                    if op == "test" {
                        return self.guard(f, entry, entries.get(&key), literal);
                    }
                    self.mutable(e, f, tx)?;
                    match op {
                        "assign" => {
                            entries.insert(key, literal(&entry[op]["value"])?);
                        }
                        "remove" => {
                            if entries.remove(&key).is_none() && !skip {
                                return Err(Fault::precondition("missing map entry"));
                            }
                        }
                        _ => return Err(Fault::unsupported("unsupported map operation")),
                    }
                }
            }
            "list" => {
                let a = data
                    .as_array_mut()
                    .ok_or_else(|| Fault::invalid("expected list"))?;
                if sel.get("append").is_some() && op == "insert" {
                    self.mutable(e, f, tx)?;
                    a.push(literal(&entry[op]["value"])?);
                } else {
                    let idx = &sel["key"]["index"];
                    let idx = idx
                        .as_i64()
                        .or_else(|| idx.as_str()?.parse().ok())
                        .and_then(|v| usize::try_from(v).ok())
                        .ok_or_else(|| Fault::unsupported("expected a list index"))?;
                    if op == "test" {
                        return self.guard(f, entry, a.get(idx), literal);
                    }
                    self.mutable(e, f, tx)?;
                    if idx >= a.len() {
                        if skip {
                            return Ok(());
                        }
                        return Err(Fault::precondition("missing list entry"));
                    }
                    match op {
                        "assign" => a[idx] = literal(&entry[op]["value"])?,
                        "remove" => self.deletes.push((f.json.clone(), idx)),
                        _ => return Err(Fault::unsupported("unsupported list operation")),
                    }
                }
            }
            _ => return Err(Fault::unsupported("field is not a collection")),
        }
        self.patch[&f.json] = data;
        Ok(())
    }
}
impl Transaction<'_> {
    pub(crate) fn apply(&mut self, e: &Entity, req: &Value) -> Result<Value> {
        let doc = &req["patch"];
        if doc["minReaderRevision"].as_u64().unwrap_or(0) > 0 {
            return Err(Fault::invalid("unsupported patch revision"));
        }
        if doc["messageType"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s != e.name)
        {
            return Err(Fault::invalid("patch is for another message type"));
        }
        let entries = doc["delta"]["entries"]
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= 4096)
            .ok_or_else(|| Fault::invalid("expected a nonempty patch delta"))?;
        let row = self.lookup(e, &req["ref"], true)?;
        let mut plan = Plan {
            row: &row,
            patch: json!({"ref":req["ref"]}),
            written: Default::default(),
            deletes: Vec::new(),
        };
        for entry in entries {
            plan.entry(self, e, entry, None, 0)?;
        }
        if plan.written.is_empty() {
            return self.image(e, row, &Value::Null);
        }
        plan.deletes.sort();
        plan.deletes.dedup();
        for (name, i) in plan.deletes.into_iter().rev() {
            let a = plan.patch[name]
                .as_array_mut()
                .ok_or_else(|| Fault::invalid("invalid list deletion"))?;
            if i >= a.len() {
                return Err(Fault::invalid("invalidated list index"));
            }
            a.remove(i);
        }
        if let Some(f) = e.fields.iter().find(|f| f.version) {
            plan.patch[&f.json] = row[&f.json].clone();
        }
        self.patch(e, &plan.patch, "Apply")
    }
}
