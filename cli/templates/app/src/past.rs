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
impl Operation {
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
type Sentence = Arc<
    dyn Fn(Reading, Value) -> Pin<Box<dyn Future<Output = Result<String, ConnectError>> + Send>>
        + Send
        + Sync,
>;
pub struct Catalog {
    default: String,
    entries: BTreeMap<(String, String), Sentence>,
    languages: BTreeSet<String>,
}
impl Catalog {
    pub fn new(default: &str) -> Self {
        Self {
            default: default.to_ascii_lowercase(),
            entries: Default::default(),
            languages: Default::default(),
        }
    }
    pub fn add<F, Fut>(&mut self, kind: &str, language: &str, format: F)
    where
        F: Fn(Reading, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, ConnectError>> + Send + 'static,
    {
        let language = language.to_ascii_lowercase();
        self.languages.insert(language.clone());
        self.entries.insert(
            (kind.into(), language),
            Arc::new(move |r, e| Box::pin(format(r, e))),
        );
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
        let format = self
            .entries
            .get(&(kind, language.clone()))
            .ok_or_else(|| ConnectError::failed_precondition("event has no sentence"))?;
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
