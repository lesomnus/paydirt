//! Bounded CEL expressions over the already-projected public WatchEvent.
use crate::{Fault, Result};
use buffa_reflect::{DescriptorPool, Kind};
use cel::{
    common::ast::{Expr, IdedExpr, LiteralValue},
    parser::Parser,
};
use serde_json::{Value, json};

pub struct Filter(Node);
enum Node {
    Const(Value),
    Field(Vec<String>),
    Has(Vec<String>),
    Type(Vec<String>),
    Not(Box<Node>),
    Binary(Op, Box<Node>, Box<Node>),
}
enum Op {
    And,
    Or,
    Eq,
    Ne,
    Starts,
    Ends,
    Contains,
}
fn invalid() -> Fault {
    Fault::invalid("unsupported or non-boolean resource CEL expression")
}
fn path(e: &IdedExpr) -> Option<Vec<String>> {
    match &e.expr {
        Expr::Ident(s) => Some(vec![s.clone()]),
        Expr::Select(s) => {
            let mut p = path(&s.operand)?;
            p.push(s.field.clone());
            Some(p)
        }
        _ => None,
    }
}
impl Filter {
    pub fn compile(text: &str, pool: &DescriptorPool) -> Result<Self> {
        if text.len() > 16 << 10 {
            return Err(Fault::invalid("resource CEL expression exceeds 16 KiB"));
        }
        let ast = Parser::new()
            .parse(text)
            .map_err(|e| Fault::invalid(e.to_string()))?;
        let (expr, boolean) = node(&ast, pool, 0)?;
        if !boolean {
            return Err(invalid());
        }
        Ok(Self(expr))
    }
    pub fn matches(&self, event: &Value, pool: &DescriptorPool) -> Result<bool> {
        self.0.eval(event, pool)?.as_bool().ok_or_else(invalid)
    }
}
fn node(e: &IdedExpr, pool: &DescriptorPool, depth: usize) -> Result<(Node, bool)> {
    if depth > 64 {
        return Err(invalid());
    }
    if let Some(p) = path(e) {
        if p.first().is_some_and(|s| s == "event") && p.len() >= 2 {
            if !matches!(
                p[1].as_str(),
                "id" | "method" | "kind" | "call_method" | "value" | "previous" | "date_created"
            ) {
                return Err(invalid());
            }
            if matches!(&e.expr,Expr::Select(s) if s.test) {
                return Ok((Node::Has(p), true));
            }
            return Ok((Node::Field(p), false));
        }
        let name = p.join(".");
        if pool.get_message_by_name(&name).is_some() {
            return Ok((Node::Const(json!(name)), false));
        }
        if let Some(v) = name.strip_prefix("payday.")
            && matches!(
                v,
                "CHANGE_KIND_CREATED"
                    | "CHANGE_KIND_UPDATED"
                    | "CHANGE_KIND_DELETED"
                    | "CHANGE_KIND_UNSPECIFIED"
            )
        {
            return Ok((Node::Const(json!(v)), false));
        }
        return Err(invalid());
    }
    if let Expr::Literal(l) = &e.expr {
        let v = match l {
            LiteralValue::Boolean(v) => json!(**v),
            LiteralValue::String(s) => json!(s.as_ref() as &str),
            LiteralValue::Bytes(b) => json!({"bytes":crate::value::encoded(b)}),
            LiteralValue::Int(i) => json!(**i),
            LiteralValue::Null => Value::Null,
            _ => return Err(invalid()),
        };
        let boolean = v.is_boolean();
        return Ok((Node::Const(v), boolean));
    }
    let Expr::Call(c) = &e.expr else {
        return Err(invalid());
    };
    if c.func_name == "type" && c.target.is_none() && c.args.len() == 1 {
        let p = path(&c.args[0])
            .filter(|p| {
                p.len() == 2 && p[0] == "event" && matches!(p[1].as_str(), "value" | "previous")
            })
            .ok_or_else(invalid)?;
        return Ok((Node::Type(p), false));
    }
    let mut args = c
        .target
        .iter()
        .map(|p| p.as_ref())
        .chain(c.args.iter())
        .map(|e| node(e, pool, depth + 1));
    let (a, ab) = args.next().ok_or_else(invalid)??;
    if c.func_name == "!_" && ab && args.next().is_none() {
        return Ok((Node::Not(Box::new(a)), true));
    }
    let (b, bb) = args.next().ok_or_else(invalid)??;
    if args.next().is_some() {
        return Err(invalid());
    }
    let op = match c.func_name.as_str() {
        "_&&_" if ab && bb => Op::And,
        "_||_" if ab && bb => Op::Or,
        "_==_" => Op::Eq,
        "_!=_" => Op::Ne,
        "startsWith" if c.target.is_some() => Op::Starts,
        "endsWith" if c.target.is_some() => Op::Ends,
        "contains" if c.target.is_some() => Op::Contains,
        _ => return Err(invalid()),
    };
    Ok((Node::Binary(op, Box::new(a), Box::new(b)), true))
}
fn lookup(event: &Value, path: &[String], pool: &DescriptorPool, defaults: bool) -> Result<Value> {
    let mut value = event;
    let mut ty = "payday.WatchEvent".to_owned();
    for name in &path[1..] {
        if let Some(full) = value["@type"].as_str().and_then(|s| s.rsplit('/').next()) {
            ty = full.into();
        }
        let desc = pool.get_message_by_name(&ty).ok_or_else(invalid)?;
        let f = desc
            .fields()
            .find(|f| f.name() == name || f.json_name() == name)
            .ok_or_else(invalid)?;
        value = &value[f.json_name()];
        if value.is_null() && !defaults {
            return Ok(Value::Null);
        }
        match f.kind() {
            Kind::Message(m) => ty = m.full_name().into(),
            Kind::Bytes => return Ok(json!({"bytes":value.as_str().unwrap_or("")})),
            Kind::String => return Ok(json!(value.as_str().unwrap_or(""))),
            _ => return Ok(value.clone()),
        }
    }
    Ok(value.clone())
}
impl Node {
    fn eval(&self, event: &Value, pool: &DescriptorPool) -> Result<Value> {
        let boolean = |v: Value| v.as_bool().ok_or_else(invalid);
        Ok(match self {
            Self::Const(v) => v.clone(),
            Self::Field(p) => lookup(event, p, pool, true)?,
            Self::Type(p) => {
                let v = lookup(event, p, pool, false)?;
                json!(v["@type"].as_str().and_then(|s| s.rsplit('/').next()))
            }
            Self::Has(p) => json!(!lookup(event, p, pool, false)?.is_null()),
            Self::Not(v) => json!(!boolean(v.eval(event, pool)?)?),
            Self::Binary(op, a, b) => {
                let a = a.eval(event, pool)?;
                match op {
                    Op::And => json!(boolean(a)? && boolean(b.eval(event, pool)?)?),
                    Op::Or => json!(boolean(a)? || boolean(b.eval(event, pool)?)?),
                    Op::Eq => json!(a == b.eval(event, pool)?),
                    Op::Ne => json!(a != b.eval(event, pool)?),
                    _ => {
                        let b = b.eval(event, pool)?;
                        let a = a.as_str().ok_or_else(invalid)?;
                        let b = b.as_str().ok_or_else(invalid)?;
                        json!(match op {
                            Op::Starts => a.starts_with(b),
                            Op::Ends => a.ends_with(b),
                            _ => a.contains(b),
                        })
                    }
                }
            }
        })
    }
}
