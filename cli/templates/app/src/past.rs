//! Application-owned history inside the current Core transaction.
use super::{Database, Session};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use connectrpc::{ConnectError, RequestContext};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

#[derive(Default)]
pub struct Options {
    pub id: Option<[u8; 16]>,
    pub subject: Option<[u8; 16]>,
    pub kind: String,
    pub reason: String,
    pub occurred: Option<DateTime<Utc>>,
}
#[derive(Clone)]
struct Cause {
    session: String,
    id: [u8; 16],
}
pub enum Begin {
    Started(Operation),
    Done([u8; 16]),
}
pub struct Operation {
    db: Database,
    ctx: RequestContext,
    pub id: [u8; 16],
}
pub fn detach(ctx: &RequestContext) -> RequestContext {
    let mut out = ctx.clone();
    out.extensions_mut().remove::<Cause>();
    out
}
pub(super) fn cause(ctx: &RequestContext) -> Option<String> {
    let s = ctx.extensions().get::<Session>()?;
    ctx.extensions()
        .get::<Cause>()
        .filter(|c| c.session == s.0)
        .map(|c| STANDARD.encode(c.id))
}
impl Database {
    pub async fn begin_event<M: buffa::Message + buffa::MessageName>(
        &self,
        ctx: &RequestContext,
        subject_entity: &str,
        payload: &M,
        opt: Options,
    ) -> Result<Begin, ConnectError> {
        let tx = ctx.extensions().get::<Session>().ok_or_else(|| {
            ConnectError::failed_precondition("begin_event needs a Core transaction")
        })?;
        let packed = buffa_types::google::protobuf::Any::pack(
            payload,
            format!("type.googleapis.com/{}", M::FULL_NAME),
        );
        let at = opt.occurred.or_else(|| {
            ctx.extensions()
                .get::<super::presence::Parties>()
                .and_then(|p| p.recorded)
        });
        let row:Value=self.rpc(ctx,subject_entity,"$begin_event",&json!({"id":opt.id.map(|v|STANDARD.encode(v)),"subject":opt.subject.map(|v|STANDARD.encode(v)),"kind":opt.kind,"reason":opt.reason,"dateStart":at.map(|d|d.to_rfc3339()),"cause":cause(ctx),"value":packed})).await?;
        let id = STANDARD
            .decode(
                row["id"]
                    .as_str()
                    .ok_or_else(|| ConnectError::internal("event ID missing"))?,
            )
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or_else(|| ConnectError::internal("invalid event ID"))?;
        if row["done"] == true {
            return Ok(Begin::Done(id));
        }
        let mut inner = ctx.clone();
        inner.extensions_mut().insert(Cause {
            session: tx.0.clone(),
            id,
        });
        Ok(Begin::Started(Operation {
            db: self.clone(),
            ctx: inner,
            id,
        }))
    }
    pub async fn statements(
        &self,
        ctx: &RequestContext,
        entity: &str,
        subject: Option<[u8; 16]>,
        event: Option<[u8; 16]>,
        known: Knowledge,
    ) -> Result<Vec<Value>, ConnectError> {
        let (ever, at) = match known {
            Knowledge::Current => (false, None),
            Knowledge::Ever => (true, None),
            Knowledge::At(t) => (false, Some(t)),
        };
        self.rpc(ctx,entity,"$statements",&json!({"subject":subject.map(|v|STANDARD.encode(v)),"event":event.map(|v|STANDARD.encode(v)),"ever":ever,"known":at})).await
    }
    pub async fn state_at<M: serde::de::DeserializeOwned>(
        &self,
        ctx: &RequestContext,
        entity: &str,
        subject: [u8; 16],
        at: DateTime<Utc>,
        known: Option<DateTime<Utc>>,
    ) -> Result<M, ConnectError> {
        let doc: Value = self
            .rpc(
                ctx,
                entity,
                "$state",
                &json!({"ref":{"id":STANDARD.encode(subject)},"at":at,"known":known}),
            )
            .await?;
        serde_json::from_value(doc["values"].clone())
            .map_err(|e| ConnectError::internal(e.to_string()))
    }
    pub async fn redact_event(
        &self,
        ctx: &RequestContext,
        entity: &str,
        id: [u8; 16],
    ) -> Result<(), ConnectError> {
        self.rpc(
            ctx,
            entity,
            "$redact_reason",
            &json!({"ref":{"id":STANDARD.encode(id)}}),
        )
        .await
    }
}
/// Event kind to fully qualified payload type, including the implicit full-name kind.
pub fn event_types() -> &'static BTreeMap<String, String> {
    static TYPES: std::sync::LazyLock<BTreeMap<String, String>> = std::sync::LazyLock::new(|| {
        super::TRAIL_POLICIES
            .values()
            .find_map(|p| p.application.get("eventTypes"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    });
    &TYPES
}
/// Decode the payload using the event's declared kind.
pub fn payload<M: buffa::MessageName + serde::de::DeserializeOwned>(
    event: &Value,
) -> Result<M, ConnectError> {
    let kind = event["kind"].as_str().unwrap_or_default();
    if event_types().get(kind).map(String::as_str) != Some(M::FULL_NAME) {
        return Err(ConnectError::invalid_argument(
            "payload does not match the declared event kind",
        ));
    }
    let mut value = event["value"].clone();
    let ty = value
        .as_object_mut()
        .and_then(|v| v.remove("@type"))
        .ok_or_else(|| ConnectError::invalid_argument("event payload has no type"))?;
    if ty.as_str().and_then(|v| v.rsplit('/').next()) != Some(M::FULL_NAME) {
        return Err(ConnectError::invalid_argument(
            "event payload type differs from its kind",
        ));
    }
    serde_json::from_value(value).map_err(|e| ConnectError::invalid_argument(e.to_string()))
}
pub enum Knowledge {
    Current,
    Ever,
    At(DateTime<Utc>),
}
pub struct Child {
    request: Value,
}
impl Child {
    pub fn new<M: buffa::Message + buffa::MessageName>(
        entity: &str,
        payload: &M,
        opt: Options,
    ) -> Self {
        let value = buffa_types::google::protobuf::Any::pack(
            payload,
            format!("type.googleapis.com/{}", M::FULL_NAME),
        );
        Self {
            request: json!({"entity":entity,"subject":opt.subject.map(|v|STANDARD.encode(v)),"id":opt.id.map(|v|STANDARD.encode(v)),"value":value,"kind":opt.kind,"dateStart":opt.occurred,"reason":opt.reason}),
        }
    }
    pub fn facts<M: serde::Serialize>(
        mut self,
        before: Option<&M>,
        after: &M,
    ) -> Result<Self, ConnectError> {
        self.request["before"] = serde_json::to_value(before)
            .map_err(|e| ConnectError::invalid_argument(e.to_string()))?;
        self.request["after"] = serde_json::to_value(after)
            .map_err(|e| ConnectError::invalid_argument(e.to_string()))?;
        Ok(self)
    }
}
impl Operation {
    async fn supersede(
        &self,
        entity: &str,
        target: [u8; 16],
        whole: bool,
        from: Option<DateTime<Utc>>,
    ) -> Result<(), ConnectError> {
        self.db.rpc(&self.ctx,entity,if whole {"$correct_event"}else{"$correct_fact"},&json!({"event":STANDARD.encode(self.id),"target":STANDARD.encode(target),"dateStart":from})).await
    }
    pub async fn correct(
        &self,
        entity: &str,
        fact: [u8; 16],
        from: DateTime<Utc>,
    ) -> Result<(), ConnectError> {
        self.supersede(entity, fact, false, Some(from)).await
    }
    pub async fn retract(&self, entity: &str, fact: [u8; 16]) -> Result<(), ConnectError> {
        self.supersede(entity, fact, false, None).await
    }
    pub async fn correct_event(
        &self,
        entity: &str,
        event: [u8; 16],
        from: DateTime<Utc>,
    ) -> Result<(), ConnectError> {
        self.supersede(entity, event, true, Some(from)).await
    }
    pub async fn retract_event(&self, entity: &str, event: [u8; 16]) -> Result<(), ConnectError> {
        self.supersede(entity, event, true, None).await
    }
    pub async fn children(
        &self,
        entity: &str,
        children: Vec<Child>,
    ) -> Result<Vec<Begin>, ConnectError> {
        let rows:Vec<Value>=self.db.rpc(&self.ctx,entity,"$event_children",&json!({"event":STANDARD.encode(self.id),"children":children.into_iter().map(|c|c.request).collect::<Vec<_>>()})).await?;
        let session = &self
            .ctx
            .extensions()
            .get::<Session>()
            .ok_or_else(|| ConnectError::failed_precondition("children require a transaction"))?
            .0;
        rows.into_iter()
            .map(|row| {
                let id = STANDARD
                    .decode(row["id"].as_str().unwrap_or_default())
                    .ok()
                    .and_then(|v| v.try_into().ok())
                    .ok_or_else(|| ConnectError::internal("invalid child identifier"))?;
                if row["done"] == true {
                    return Ok(Begin::Done(id));
                }
                let mut ctx = self.ctx.clone();
                ctx.extensions_mut().insert(Cause {
                    session: session.clone(),
                    id,
                });
                Ok(Begin::Started(Operation {
                    db: self.db.clone(),
                    ctx,
                    id,
                }))
            })
            .collect()
    }

    pub fn context(&self) -> &RequestContext {
        &self.ctx
    }
    pub async fn facts<M: serde::Serialize>(
        &self,
        entity: &str,
        before: &M,
        after: &M,
        valid_from: Option<DateTime<Utc>>,
    ) -> Result<(), ConnectError> {
        let _:Value=self.db.rpc(&self.ctx,entity,"$event_facts",&json!({"event":STANDARD.encode(self.id),"before":before,"after":after,"dateStart":valid_from.map(|t|t.to_rfc3339())})).await?;
        Ok(())
    }
}
/// The formatter can use the caller's scoped sinks to resolve today's names.
#[derive(Clone)]
pub struct Reading {
    pub ctx: RequestContext,
    pub language: String,
    pub zone: Tz,
}
impl Reading {
    /// Resolve a live display name under this request's wall and attribute it to its owner.
    pub async fn name(
        &self,
        db: &Database,
        entity: &str,
        id: [u8; 16],
    ) -> Result<String, ConnectError> {
        let row: Value = db
            .rpc(
                &self.ctx,
                entity,
                "Get",
                &json!({"ref":{"id":STANDARD.encode(id)}}),
            )
            .await?;
        let owner = row["tenant"]["id"]
            .as_str()
            .or_else(|| row["tenantId"].as_str());
        if let Some(tenant) = owner
            .and_then(|s| STANDARD.decode(s).ok())
            .and_then(|v| v.try_into().ok())
        {
            self.named(tenant, id);
        }
        Ok(row["label"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| row["alias"].as_str())
            .unwrap_or_default()
            .to_owned())
    }
    /// Record the current owner and subject of a name actually used in the sentence.
    pub fn named(&self, tenant: [u8; 16], subject: [u8; 16]) {
        super::access::Attempt::named(&self.ctx, &tenant, &subject);
    }
}
type Sentence = Arc<
    dyn Fn(Reading, Value) -> Pin<Box<dyn Future<Output = Result<String, ConnectError>> + Send>>
        + Send
        + Sync,
>;
pub struct Catalog {
    default: String,
    entries: BTreeMap<(String, String), Sentence>,
    languages: BTreeSet<String>,
    legacy: Option<Sentence>,
}
impl Catalog {
    pub fn new(default: &str) -> Self {
        Self {
            default: default.to_ascii_lowercase(),
            entries: Default::default(),
            languages: Default::default(),
            legacy: None,
        }
    }
    pub fn add<F, Fut>(&mut self, kind: &str, language: &str, format: F)
    where
        F: Fn(Reading, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, ConnectError>> + Send + 'static,
    {
        let language = language.to_ascii_lowercase();
        assert!(
            !self
                .entries
                .contains_key(&(kind.to_owned(), language.clone())),
            "duplicate event sentence for {kind} in {language}"
        );
        self.languages.insert(language.clone());
        self.entries.insert(
            (kind.into(), language),
            Arc::new(move |r, e| Box::pin(format(r, e))),
        );
    }
    pub fn add_payload<M, F, Fut>(
        &mut self,
        kind: Option<&str>,
        language: &str,
        format: F,
    ) -> Result<(), String>
    where
        M: buffa::MessageName,
        F: Fn(Reading, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, ConnectError>> + Send + 'static,
    {
        let declared: Vec<_> = event_types()
            .iter()
            .filter(|(_, ty)| ty.as_str() == M::FULL_NAME)
            .map(|(k, _)| k.as_str())
            .collect();
        let selected = match kind {
            Some(k) if declared.contains(&k) => k,
            None if declared.len() == 1 => declared[0],
            _ => return Err(format!("expected a declared kind of {}", M::FULL_NAME)),
        };
        self.add(selected, language, format);
        Ok(())
    }
    pub async fn describe_event(
        &self,
        ctx: &RequestContext,
        mut event: Value,
        language: &str,
        zone: Tz,
    ) -> Result<Value, ConnectError> {
        event["desc"] = json!(self.describe(ctx, event.clone(), language, zone).await?);
        Ok(event)
    }
    pub fn legacy<F, Fut>(&mut self, format: F)
    where
        F: Fn(Reading, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, ConnectError>> + Send + 'static,
    {
        self.legacy = Some(Arc::new(move |r, e| Box::pin(format(r, e))));
    }
    pub fn unsaid(&self, language: &str, kinds: &[&str]) -> Vec<String> {
        kinds
            .iter()
            .filter(|k| {
                !self
                    .entries
                    .contains_key(&(k.to_string(), language.to_ascii_lowercase()))
            })
            .map(|k| k.to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub fn validate(&self, kinds: &[&str]) -> Result<(), String> {
        if !self.languages.contains(&self.default) {
            return Err("missing default event language".into());
        }
        for k in kinds {
            for l in &self.languages {
                if !self.entries.contains_key(&(k.to_string(), l.clone())) {
                    return Err(format!("missing {k} sentence in {l}"));
                }
            }
        }
        Ok(())
    }
    pub async fn describe(
        &self,
        ctx: &RequestContext,
        event: Value,
        preferences: &str,
        zone: Tz,
    ) -> Result<String, ConnectError> {
        let mut languages: Vec<_> = preferences
            .split(',')
            .enumerate()
            .filter_map(|(i, part)| {
                let mut pair = part.trim().split(';');
                let language = pair.next()?.trim().to_ascii_lowercase();
                let weight = match pair.next() {
                    None => 1.0,
                    Some(q) => q.trim().strip_prefix("q=")?.parse::<f32>().ok()?,
                };
                (weight > 0.0 && weight <= 1.0).then_some((weight, i, language))
            })
            .collect();
        languages.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let language = languages
            .into_iter()
            .find_map(|(_, _, mut l)| {
                loop {
                    if self.languages.contains(&l) {
                        break Some(l);
                    }
                    if let Some(pos) = l.rfind('-') {
                        l.truncate(pos)
                    } else {
                        break None;
                    }
                }
            })
            .unwrap_or_else(|| self.default.clone());
        let kind = event["kind"].as_str().unwrap_or_default().to_owned();
        let preferred = self.entries.get(&(kind.clone(), language.clone()));
        let (format, language) = if let Some(f) = preferred {
            (f, language)
        } else if let Some(f) = self.entries.get(&(kind.clone(), self.default.clone())) {
            (f, self.default.clone())
        } else if let Some(f) = &self.legacy {
            (f, language)
        } else {
            return Ok(event["said"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(&kind)
                .to_string());
        };
        format(
            Reading {
                ctx: ctx.clone(),
                language,
                zone,
            },
            event,
        )
        .await
    }
}
