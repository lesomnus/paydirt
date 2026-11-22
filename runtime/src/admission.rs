//! Application-owned checks for individual writes in the SQL transaction.
use crate::{Call, Fault, Frame, Policies, Result};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct Decision {
    pub entity: String,
    pub object: Value,
    pub method: String,
    pub by: String,
    pub frame: Option<Frame>,
    pub request: Value,
    pub previous: Option<Value>,
    pub value: Option<Value>,
}
pub struct Options {
    pub entities: Vec<String>,
    pub secrets: BTreeMap<String, Vec<u32>>,
    pub timeout: Duration,
    pub limit: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            entities: vec![],
            secrets: Default::default(),
            timeout: Duration::from_secs(2),
            limit: 64,
        }
    }
}
#[derive(Deserialize)]
struct Field {
    number: u32,
    name: String,
    #[serde(default)]
    target: String,
    #[serde(default)]
    secret: bool,
}
#[derive(Deserialize)]
struct Message {
    fields: Vec<Field>,
    map: bool,
}
type Rule = dyn Fn(Decision) -> Result<()> + Send + Sync;
pub(crate) struct Rules {
    entities: BTreeSet<String>,
    schema: BTreeMap<String, Message>,
    policy: Arc<Rule>,
    timeout: Duration,
    limit: usize,
    running: Arc<std::sync::atomic::AtomicUsize>,
}

impl Rules {
    pub(crate) fn new(
        schema: &str,
        policies: &Policies,
        opts: Options,
        policy: Arc<Rule>,
    ) -> Result<Self> {
        let mut schema: BTreeMap<String, Message> =
            serde_json::from_str(schema).map_err(|e| Fault::internal(e.to_string()))?;
        for (name, numbers) in opts.secrets {
            let ty = schema
                .get_mut(&name)
                .ok_or_else(|| Fault::invalid(format!("unknown admission message {name}")))?;
            for n in numbers {
                let f = ty
                    .fields
                    .iter_mut()
                    .find(|v| v.number == n)
                    .ok_or_else(|| Fault::invalid(format!("unknown field {name}.{n}")))?;
                f.secret = true;
            }
        }
        if opts.entities.is_empty() || opts.timeout.is_zero() || opts.limit == 0 {
            return Err(Fault::invalid(
                "admission needs entities, a positive timeout and limit",
            ));
        }
        let mut entities = BTreeSet::new();
        for selector in &opts.entities {
            let (pkg, short) = selector
                .rsplit_once('.')
                .ok_or_else(|| Fault::invalid("expected qualified admission selector"))?;
            let mut matched = false;
            for (name, p) in policies {
                if p.entity.is_null()
                    || !matches!(
                        p.entity["own"].as_str(),
                        None | Some("OWN_UNSPECIFIED" | "OWN_TENANT" | "OWN_HOLDER")
                    )
                {
                    continue;
                }
                let Some((a, b)) = name.rsplit_once('.') else {
                    continue;
                };
                if (pkg == "*" || pkg == a) && (short == "*" || short == b) {
                    matched = true;
                    entities.insert(name.clone());
                }
            }
            if !matched {
                return Err(Fault::invalid(format!(
                    "no eligible entity matches {selector}"
                )));
            }
        }
        Ok(Self {
            entities,
            schema,
            policy,
            timeout: opts.timeout,
            limit: opts.limit,
            running: Default::default(),
        })
    }
    fn clean(&self, ty: &str, value: &Value) -> Value {
        if value.is_null() {
            return Value::Null;
        }
        if let Some(values) = value.as_array() {
            return Value::Array(values.iter().map(|v| self.clean(ty, v)).collect());
        }
        if ty.is_empty() {
            return value.clone();
        }
        if ty == "google.protobuf.Any" || ty.starts_with("patch.") {
            return Value::Null;
        }
        // Scalar WKT JSON representations contain no named message fields.
        if matches!(
            ty,
            "google.protobuf.Timestamp" | "google.protobuf.Duration" | "google.protobuf.FieldMask"
        ) {
            return value.clone();
        }
        let Some(m) = self.schema.get(ty) else {
            return Value::Null;
        };
        let Some(raw) = value.as_object() else {
            return Value::Null;
        };
        let mut out = Map::new();
        if m.map {
            if let Some(field) = m.fields.iter().find(|f| f.number == 2 && !f.secret) {
                for (k, v) in raw {
                    out.insert(k.clone(), self.clean(&field.target, v));
                }
            }
        } else {
            for field in &m.fields {
                if !field.secret
                    && let Some(v) = raw.get(&field.name)
                {
                    let v = self.clean(&field.target, v);
                    if !v.is_null() {
                        out.insert(field.name.clone(), v);
                    }
                }
            }
        }
        Value::Object(out)
    }
    pub(crate) fn check(
        &self,
        policies: &Policies,
        e: &str,
        op: &str,
        id: &Value,
        frame: Option<&Frame>,
        origin: Option<&Call>,
        before: Option<&Value>,
        after: Option<&Value>,
    ) -> Result<()> {
        if !self.entities.contains(e) {
            return Ok(());
        }
        if origin.is_none() {
            return Err(Fault::precondition(
                "admitted mutations require the originating application call",
            ));
        }
        let by = format!("/{e}Service/{op}");
        let request = origin
            .and_then(|c| {
                let ty = policies.get(&c.entity)?.methods.get(&c.method).and_then(|m|m["input"].as_str());
                if ty.is_none() && c.method=="AddMany" {
                    let items=c.request["items"].as_array()?;
                    let item_type=format!("{}AddRequest",c.entity);
                    return Some(serde_json::json!({"items": items.iter().map(|v|self.clean(&item_type,v)).collect::<Vec<_>>()}));
                }
                let ty=ty?;
                Some(self.clean(ty.trim_start_matches('.'), &c.request))
            })
            .unwrap_or(Value::Null);
        let call = Decision {
            entity: e.into(),
            object: id.clone(),
            method: frame
                .map(|f| f.action.clone())
                .or_else(|| origin.map(|c| format!("/{}Service/{}", c.entity, c.method)))
                .unwrap_or_else(|| by.clone()),
            by,
            frame: frame.cloned().map(|mut f| {
                for row in [&mut f.row, &mut f.caller_row].into_iter().flatten() {
                    row.value = self.clean(&row.entity, &row.value);
                }
                f
            }),
            request,
            previous: before.map(|v| self.clean(e, v)),
            value: after.map(|v| self.clean(e, v)),
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::sync::{atomic::Ordering, mpsc};
            if self
                .running
                .try_update(Ordering::AcqRel, Ordering::Relaxed, |n| {
                    (n < self.limit).then_some(n + 1)
                })
                .is_err()
            {
                return Err(Fault::exhausted("admission limit reached"));
            }
            let policy = self.policy.clone();
            let running = self.running.clone();
            let (tx, rx) = mpsc::sync_channel(1);
            let result = std::thread::Builder::new()
                .name("paydirt-admission".into())
                .spawn(move || {
                    let decision =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| policy(call)))
                            .unwrap_or_else(|_| Err(Fault::unavailable("admission policy failed")));
                    running.fetch_sub(1, Ordering::Release);
                    let _ = tx.send(decision);
                });
            if result.is_err() {
                self.running.fetch_sub(1, Ordering::Release);
                return Err(Fault::unavailable("admission worker unavailable"));
            }
            rx.recv_timeout(self.timeout).map_err(|e| match e {
                mpsc::RecvTimeoutError::Timeout => Fault::deadline("admission timed out"),
                _ => Fault::unavailable("admission unavailable"),
            })?
        }
        #[cfg(target_arch = "wasm32")]
        {
            (self.policy)(call)
        }
    }
}
