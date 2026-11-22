use crate::{
    Fault, Result, Scope, Transaction,
    driver::{Bind, Driver, Query},
    value,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use hmac::{Hmac, Mac};
use paydirt_model::{Dialect, Entity, quote};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Trusted per-principal topic policy. A grant is a canonical n:name, r:UUID,
/// "self" for the actor's resource, or an explicit wildcard.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TopicGrants {
    pub read: Vec<String>,
    pub publish: Vec<String>,
    pub producer: Option<String>,
}

/// Application-owned topic policy. The reader and grants are resolved again on
/// each call, including stream invalidation. Relationships are read on the same
/// connection and under the same scope as the outer operation.
pub trait Policy: Send + Sync + 'static {
    fn resolve(
        &self,
        rows: &mut Rows<'_, '_>,
        frame: &crate::Frame,
        tenant: &[u8],
        method: &str,
        request: &Value,
    ) -> Result<(Vec<u8>, TopicGrants)>;
}
pub struct Rows<'a, 'db>(&'a mut Transaction<'db>);
impl Rows<'_, '_> {
    pub fn get(&mut self, entity: &str, reference: &Value) -> Result<Option<Value>> {
        let e = self.0.model.entity(entity).map_err(Fault::invalid)?;
        match self.0.lookup(e, reference, false) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.code == "not_found" => Ok(None),
            Err(e) => Err(e),
        }
    }
}
impl crate::Store {
    pub fn with_notification_policy(mut self, policy: impl Policy) -> Self {
        self.topic_policy = Some(Box::new(policy));
        self
    }
}
pub(crate) fn install(db: &mut dyn Driver) -> Result<()> {
    db.batch("CREATE TABLE IF NOT EXISTS paydirt_notify_key(id INTEGER PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS paydirt_notify_revisions(tenant TEXT NOT NULL,topic TEXT NOT NULL,reader TEXT NOT NULL,version BIGINT NOT NULL,PRIMARY KEY(tenant,topic,reader));")?;
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| Fault::internal(e.to_string()))?;
    let mut q = Query::new("");
    let k = q.param(Bind::Text(hex::encode(key)), db.dialect());
    q.sql = format!(
        "INSERT INTO paydirt_notify_key (id,value) VALUES(1,{k}) ON CONFLICT(id) DO NOTHING"
    );
    db.exec(q)?;
    Ok(())
}
fn text(v: &Value, len: usize) -> Result<&str> {
    v.as_str()
        .filter(|s| !s.is_empty() && s.len() <= len && !s.contains('\0'))
        .ok_or_else(|| Fault::invalid("invalid notification text"))
}
fn id(v: &Value) -> Result<Vec<u8>> {
    let bytes = value::bytes(v)?;
    if bytes.len() != 16 || bytes[9] == 0 || !paydirt::has_domain(&bytes, bytes[9]) {
        return Err(Fault::invalid("invalid notification identifier"));
    }
    Ok(bytes)
}
fn topic(v: &Value) -> Result<String> {
    match (v.get("name"), v.get("resourceId")) {
        (Some(n), None) => Ok(format!("n:{}", text(n, 256)?)),
        (None, Some(r)) => Ok(format!("r:{}", paydirt::format_id(&id(r)?).unwrap())),
        _ => Err(Fault::invalid("one topic key required")),
    }
}
fn from_topic(key: &str) -> Result<Value> {
    if let Some(name) = key.strip_prefix("n:") {
        return Ok(json!({"name":name}));
    }
    key.strip_prefix("r:")
        .and_then(paydirt::parse_id)
        .map(|i| json!({"resourceId":value::encoded(&i)}))
        .ok_or_else(|| Fault::internal("invalid stored topic"))
}
fn visible(grants: &[String], key: &str, actor: &[u8]) -> bool {
    grants.iter().any(|g| {
        g == "*"
            || g == key
            || (g == "self"
                && key
                    .strip_prefix("r:")
                    .and_then(paydirt::parse_id)
                    .is_some_and(|id| id == actor))
    })
}
fn notice(row: &Value) -> Result<Value> {
    let mut n = json!({"id":row["id"],"kind":row["kind"],"topic":from_topic(text(&row["topicKey"],280)?)?,"dateCreated":row["dateCreated"],"dateUpdated":row["dateUpdated"]});
    if row["tenantId"] != value::encoded(&[0; 16]) {
        n["tenantId"] = row["tenantId"].clone()
    }
    if let Some(v) = row.get("sourceId") {
        n["sourceId"] = v.clone()
    }
    if row["payloadPresent"] == true {
        n["payload"] = json!({"@type":row["payloadType"],"value":row["payloadValue"]})
    }
    Ok(n)
}
fn receipt(row: &Value) -> Value {
    let mut r = row.clone();
    if r["tenantId"] == value::encoded(&[0; 16]) {
        r.as_object_mut().unwrap().remove("tenantId");
    }
    r
}
fn seq(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

struct Reader<'a> {
    namespace: &'a str,
    tenant: Value,
    actor: Vec<u8>,
    grants: &'a TopicGrants,
}
impl Transaction<'_> {
    pub(crate) fn notify(&mut self, service: &str, method: &str, req: &Value) -> Result<Value> {
        let cfg = &self
            .policies
            .values()
            .find(|p| p.entity["own"] == "OWN_HOLDER")
            .ok_or_else(|| Fault::internal("no application schema"))?
            .application;
        let accounts = service.ends_with(".NotificationAccount");
        let global = accounts || cfg["notify"].get("global").is_some();
        let internal = matches!(self.scope, Scope::Internal) && self.frame.is_none();
        let host = TopicGrants {
            publish: vec!["*".into()],
            producer: Some("deployment".into()),
            ..Default::default()
        };
        let frame = self.frame;
        if accounts
            && !internal
            && !frame.is_some_and(|f| f.tenant.is_empty() && f.account.is_some())
        {
            return Err(Fault::denied("account namespace requires an account frame"));
        }
        let found = if !global && method == "Get" {
            let key = &req["ref"]["id"];
            id(key)?;
            let records = self.owned_entity("OWN_NOTIFICATION")?.clone();
            Some(self.lookup(&records, &json!({"id":key}), false)?)
        } else {
            None
        };
        let requested = req
            .get("tenantId")
            .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()));
        let tenant = if global {
            if requested.is_some() {
                return Err(Fault::invalid("this namespace names no tenant"));
            }
            vec![0; 16]
        } else {
            let t = id(found
                .as_ref()
                .map(|row| &row["tenantId"])
                .or(requested)
                .ok_or_else(|| Fault::invalid("tenant ID required"))?)?;
            if !paydirt::has_domain(&t, 1) {
                return Err(if found.is_some() {
                    Fault::missing()
                } else {
                    Fault::invalid("not a tenant ID")
                });
            }
            if self.scope.tenants().is_some_and(|ids| !ids.contains(&t)) {
                return Err(Fault::denied("outside tenant scope"));
            }
            t
        };
        let (actor, grants) = match (frame, self.topic_policy) {
            (None, _) if internal && method == "Add" => (vec![0; 16], host),
            (Some(f), Some(policy)) => {
                let (reader, grants) = policy.resolve(&mut Rows(self), f, &tenant, method, req)?;
                if reader.len() != 16 || reader.iter().all(|&b| b == 0) {
                    return Err(Fault::precondition("topic policy returned an empty reader"));
                }
                (reader, grants)
            }
            (Some(f), None) => (
                f.actor.clone().ok_or_else(|| {
                    Fault::precondition("notification requires a reader identity")
                })?,
                f.topics.clone(),
            ),
            _ => return Err(Fault::unauthenticated("notification requires a caller")),
        };
        if let Some(row) = found {
            if !visible(&grants.read, row["topicKey"].as_str().unwrap_or(""), &actor) {
                return Err(Fault::missing());
            }
            return notice(&row);
        }
        let r = Reader {
            namespace: cfg["goPackage"]
                .as_str()
                .unwrap_or(service.split('.').next().unwrap_or(service))
                .split(';')
                .next()
                .unwrap_or(service),
            tenant: value::encoded(&tenant),
            actor,
            grants: &grants,
        };
        let scope = Scope::Tenant(tenant);
        let mut db = Transaction {
            admission: None,
            origin: None,
            seen: None,
            changes: self.changes,
            outbox: self.outbox,
            resources: self.resources,
            topic_policy: self.topic_policy,
            inputs: self.inputs,
            namer: self.namer,
            db: self.db,
            model: self.model,
            policies: self.policies,
            scope: &scope,
            frame,
            failed: None,
        };
        let record = db.owned_entity("OWN_NOTIFICATION")?.clone();
        let receipts = db.owned_entity("OWN_NOTIFICATION_RECEIPT")?.clone();
        match method {
            "Add" => db.publish(&record, &r, req, &cfg["notificationPayloads"]),
            "Get" => notice(&db.visible_notice(&record, &r, &req["ref"]["id"])?),
            "Mark" => db.mark(&record, &receipts, &r, req),
            "Read" | "List" | "Count" | "$listen" | "$listen_interest" => {
                db.inbox(service, method, &record, &receipts, &r, req)
            }
            _ => Err(Fault::unsupported("unknown notification method")),
        }
    }
    fn notice_lock(&mut self, scope: &Value) -> Result<()> {
        if self.db.dialect() == Dialect::Postgres {
            let mut q = Query::new("");
            let key = q.param(Bind::Text(scope.to_string()), self.db.dialect());
            q.sql = format!(
                "SELECT CAST(pg_advisory_xact_lock(hashtextextended({key},0)) AS TEXT) AS value"
            );
            self.db.rows(q)?;
        }
        Ok(())
    }
    fn publish(
        &mut self,
        e: &Entity,
        r: &Reader<'_>,
        req: &Value,
        payloads: &Value,
    ) -> Result<Value> {
        let key = topic(&req["topic"])?;
        let kind = text(&req["kind"], 128)?;
        let idem = text(&req["idempotencyKey"], 256)?;
        if !visible(&r.grants.publish, &key, &r.actor) {
            return Err(Fault::denied("topic publication denied"));
        }
        let producer = r
            .grants
            .producer
            .as_deref()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or_else(|| Fault::precondition("no stable producer"))?;
        let mut row = json!({"tenantId":r.tenant,"topicKey":key,"kind":kind,"producer":producer,"idempotencyKey":idem,"payloadPresent":false});
        if let Some(v) = req.get("sourceId").filter(|v| v.as_str() != Some("")) {
            id(v)?;
            row["sourceId"] = v.clone()
        }
        if let Some(any) = req.get("payload").filter(|v| !v.is_null()) {
            let url = text(&any["@type"], 1024)?;
            if !seq(payloads)
                .iter()
                .any(|v| v.as_str() == url.rsplit('/').next())
            {
                return Err(Fault::invalid("undeclared notification payload"));
            }
            value::bytes(&any["value"])?;
            row["payloadType"] = json!(url);
            row["payloadValue"] = any["value"].clone();
            row["payloadPresent"] = json!(true);
        }
        let lookup = [
            ("tenant_id", &r.tenant),
            ("producer", &row["producer"]),
            ("idempotency_key", &row["idempotencyKey"]),
        ];
        self.notice_lock(&json!([e.name, r.tenant, producer, idem]))?;
        if let Some(old) = self.matching(e, &lookup, false)?.pop() {
            for field in [
                "topicKey",
                "kind",
                "sourceId",
                "payloadPresent",
                "payloadType",
                "payloadValue",
            ] {
                if old[field] != row[field] && !(row[field].is_null() && old[field] == "") {
                    return Err(Fault::conflict(
                        "idempotency key identifies different content",
                    ));
                }
            }
            return notice(&old);
        }
        let made = self.insert(e, &row)?;
        let raw = self.lookup(e, &json!({"id":made["id"]}), false)?;
        self.bump(r, &key, "")?;
        notice(&raw)
    }
    fn visible_notice(&mut self, e: &Entity, r: &Reader<'_>, v: &Value) -> Result<Value> {
        id(v)?;
        let row = self.lookup(e, &json!({"id":v}), false)?;
        if row["tenantId"] != r.tenant
            || !visible(
                &r.grants.read,
                row["topicKey"].as_str().unwrap_or(""),
                &r.actor,
            )
        {
            return Err(Fault::missing());
        }
        Ok(row)
    }
    fn mark(
        &mut self,
        e: &Entity,
        receipts: &Entity,
        r: &Reader<'_>,
        req: &Value,
    ) -> Result<Value> {
        let refs = seq(&req["refs"]);
        if refs.is_empty() || refs.len() > 100 {
            return Err(Fault::invalid("Mark needs 1 to 100 references"));
        }
        let actor = value::encoded(&r.actor);
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();
        for reference in refs {
            let v = &reference["id"];
            if !seen.insert(id(v)?) {
                return Err(Fault::invalid("duplicate Mark reference"));
            }
        }
        // Serialize overlapping batches in a single order, then return request order.
        for key in seen {
            self.lookup(e, &json!({"id":value::encoded(&key)}), true)?;
            self.notice_lock(&json!([receipts.name, actor, value::encoded(&key)]))?;
        }
        for reference in refs {
            rows.push(self.visible_notice(e, r, &reference["id"])?);
        }
        let mut out = Vec::new();
        for row in rows {
            let n = &row["id"];
            let found = self.matching(
                receipts,
                &[("notification_id", n), ("reader_id", &actor)],
                false,
            )?;
            let v = if let Some(old) = found.first() {
                old.clone()
            } else {
                let v=self.insert(receipts,&json!({"tenantId":r.tenant,"notificationId":n,"readerId":actor,"dateRead":self.inputs.stamp()}))?;
                self.bump(
                    r,
                    row["topicKey"].as_str().unwrap(),
                    actor.as_str().unwrap(),
                )?;
                v
            };
            out.push(receipt(&v));
        }
        Ok(json!({"items":out,"readerId":actor}))
    }
    fn notify_route(r: &Reader<'_>, topic: &str, reader: &str) -> Result<String> {
        let tenant = paydirt::format_id(&value::bytes(&r.tenant)?)
            .ok_or_else(|| Fault::internal("invalid stored notification tenant"))?;
        let who = if reader.is_empty() {
            "00000000-0000-0000-0000-000000000000".to_owned()
        } else {
            paydirt::format_id(&value::bytes(&json!(reader))?)
                .ok_or_else(|| Fault::internal("invalid notification reader"))?
        };
        Ok(paydirt::watch::route_key(&[
            "payday.notify",
            r.namespace,
            &tenant,
            topic,
            &who,
        ]))
    }
    fn bump(&mut self, r: &Reader<'_>, topic: &str, reader: &str) -> Result<()> {
        let mut q = Query::new("");
        let d = self.db.dialect();
        let parts =
            [r.tenant.as_str().unwrap(), topic, reader].map(|s| q.param(Bind::Text(s.into()), d));
        q.sql = format!(
            "INSERT INTO paydirt_notify_revisions(tenant,topic,reader,version) VALUES({},{},{},1) ON CONFLICT(tenant,topic,reader) DO UPDATE SET version=paydirt_notify_revisions.version+1",
            parts[0], parts[1], parts[2]
        );
        self.db.exec(q)?;
        let by = "/payday.notify/internal/Invalidate";
        let nil = vec![0; 16];
        let tenant = paydirt::format_id(&value::bytes(&r.tenant)?)
            .ok_or_else(|| Fault::internal("invalid tenant"))?;
        let reader_id = if reader.is_empty() {
            nil.clone()
        } else {
            value::bytes(&json!(reader))?
        };
        let who =
            paydirt::format_id(&reader_id).ok_or_else(|| Fault::internal("invalid reader"))?;
        let topic_value = if let Some(name) = topic.strip_prefix("n:") {
            json!({"Name":name,"Resource":paydirt::format_id(&nil)})
        } else {
            json!({"Name":"","Resource":topic.trim_start_matches("r:")})
        };
        let signal = value::encoded(
            &serde_json::to_vec(&json!({"Tenant":tenant,"Topics":[topic_value],"Reader":who}))
                .map_err(|e| Fault::internal(e.to_string()))?,
        );
        let routes = vec![Self::notify_route(r, topic, reader)?];
        self.changes.push(paydirt::watch::Change {
            by: by.into(),
            method: by.into(),
            key: tenant,
            routes: Some(routes.clone()),
            signal: signal.as_str().unwrap().into(),
            ..Default::default()
        });
        if self.outbox {
            let e = self.owned_entity("OWN_OUTBOX")?.clone();
            let mut queue = Transaction {
                admission: None,
                origin: None,
                seen: None,
                db: self.db,
                model: self.model,
                policies: self.policies,
                resources: self.resources,
                topic_policy: self.topic_policy,
                inputs: self.inputs,
                namer: self.namer,
                outbox: false,
                changes: self.changes,
                scope: &Scope::Internal,
                frame: None,
                failed: None,
            };
            queue.insert(&e,&json!({"actorId":value::encoded(&nil),"tenantId":r.tenant,"method":by,"by":by,"objectId":r.tenant,
                "routes":value::encoded(&serde_json::to_vec(&routes).map_err(|e|Fault::internal(e.to_string()))?),"signal":signal}))?;
        }
        Ok(())
    }
    fn inbox(
        &mut self,
        service: &str,
        method: &str,
        e: &Entity,
        receipts: &Entity,
        r: &Reader<'_>,
        req: &Value,
    ) -> Result<Value> {
        let mut topics = BTreeMap::new();
        let requested = seq(&req["topics"]);
        if requested.is_empty() || requested.len() > 100 {
            return Err(Fault::invalid("request needs 1 to 100 topics"));
        }
        for v in requested {
            let k = topic(v)?;
            if !visible(&r.grants.read, &k, &r.actor) {
                return Err(Fault::denied("topic outside reader policy"));
            }
            if topics.insert(k, v.clone()).is_some() {
                return Err(Fault::invalid("duplicate topic"));
            }
        }
        let actor = value::encoded(&r.actor);
        if method == "$listen_interest" {
            let mut routes = Vec::new();
            for t in topics.keys() {
                routes.push(Self::notify_route(r, t, "")?);
                routes.push(Self::notify_route(r, t, actor.as_str().unwrap())?);
            }
            return Ok(json!(routes));
        }
        if method == "$listen" {
            let mut revisions = Vec::new();
            for k in topics.keys() {
                let mut q = Query::new("");
                let d = self.db.dialect();
                let args = [r.tenant.as_str().unwrap(), k, actor.as_str().unwrap()]
                    .map(|s| q.param(Bind::Text(s.into()), d));
                q.sql = format!(
                    "SELECT CAST(COALESCE(SUM(version),0) AS TEXT) AS value FROM paydirt_notify_revisions WHERE tenant={} AND topic={} AND reader IN ('',{})",
                    args[0], args[1], args[2]
                );
                revisions.push(self.db.rows(q)?[0].clone());
            }
            return Ok(
                json!({"revisions":revisions,"topics":topics.values().collect::<Vec<_>>(),"readerId":actor}),
            );
        }
        let filter = &req["filter"];
        let unread = filter["unreadOnly"] == true;
        let kinds = seq(&filter["kinds"])
            .iter()
            .map(|v| text(v, 128))
            .collect::<Result<BTreeSet<_>>>()?;
        let sources = seq(&filter["sourceIds"])
            .iter()
            .map(id)
            .collect::<Result<BTreeSet<_>>>()?;
        if seq(&filter["kinds"]).len() > 100 || seq(&filter["sourceIds"]).len() > 100 {
            return Err(Fault::invalid("too many kinds or sources"));
        }
        let mut scope_hash = Sha256::new();
        let f = self
            .frame
            .ok_or_else(|| Fault::unauthenticated("notification requires a caller"))?;
        scope_hash.update(
            json!([
                service,
                method,
                r.tenant,
                actor,
                topics.keys().collect::<Vec<_>>(),
                kinds,
                sources,
                unread,
                f.actor,
                f.caller,
                f.account,
                f.tenants,
                f.sets,
                f.topics.read
            ])
            .to_string(),
        );
        let scope = hex::encode(scope_hash.finalize());
        let mut q = Query::new("");
        let d = self.db.dialect();
        let mut predicates = self.wall(e, &mut q)?;
        let col = |entity: &Entity, name: &str| {
            entity
                .field(name)
                .map(|f| quote(&f.column()))
                .map_err(Fault::internal)
        };
        let tenant = q.param(Bind::Bytes(value::bytes(&r.tenant)?), d);
        predicates.push(format!("r.{}={tenant}", col(e, "tenant_id")?));
        let keys: Vec<_> = topics
            .keys()
            .map(|k| q.param(Bind::Text(k.clone()), d))
            .collect();
        predicates.push(format!(
            "r.{} IN ({})",
            col(e, "topic_key")?,
            keys.join(",")
        ));
        if !kinds.is_empty() {
            let vs: Vec<_> = kinds
                .iter()
                .map(|k| q.param(Bind::Text(k.to_string()), d))
                .collect();
            predicates.push(format!("r.{} IN ({})", col(e, "kind")?, vs.join(",")))
        }
        if !sources.is_empty() {
            let vs: Vec<_> = sources
                .iter()
                .map(|k| q.param(Bind::Bytes(k.clone()), d))
                .collect();
            predicates.push(format!("r.{} IN ({})", col(e, "source_id")?, vs.join(",")))
        }
        if unread {
            let who = q.param(Bind::Bytes(r.actor.clone()), d);
            predicates.push(format!(
                "NOT EXISTS (SELECT 1 FROM {} s WHERE s.{}=r.{} AND s.{}={who})",
                quote(&receipts.table()),
                col(receipts, "notification_id")?,
                col(e, "id")?,
                col(receipts, "reader_id")?
            ))
        }
        let size = req["size"].as_i64().unwrap_or(0);
        if size < 0 {
            return Err(Fault::invalid("negative page size"));
        }
        let size = if size == 0 { 50 } else { size.min(100) } as usize;
        let key = self
            .db
            .rows(Query::new(
                "SELECT value FROM paydirt_notify_key WHERE id=1",
            ))?
            .pop()
            .ok_or_else(|| Fault::internal("missing notification cursor key"))?;
        let mac =
            || Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("HMAC accepts any key size");
        if let Some(c) = req["cursor"].as_str().filter(|s| !s.is_empty()) {
            let bad = || Fault::invalid("invalid notification cursor");
            if c.len() > 8192 {
                return Err(bad());
            }
            let (body, sig) = c.split_once('.').ok_or_else(bad)?;
            let b = B64.decode(body).map_err(|_| bad())?;
            let mut h = mac();
            h.update(&b);
            h.verify_slice(&B64.decode(sig).map_err(|_| bad())?)
                .map_err(|_| bad())?;
            let pos: Value = serde_json::from_slice(&b).map_err(|_| bad())?;
            if pos["scope"] != scope {
                return Err(bad());
            }
            let stamp = q.param(Bind::Text(text(&pos["dateCreated"], 64)?.into()), d);
            let ident = q.param(Bind::Bytes(id(&pos["id"])?), d);
            predicates.push(format!(
                "(r.{},r.{})<({stamp},{ident})",
                col(e, "date_created")?,
                col(e, "id")?
            ));
        }
        let limit = if method == "Count" {
            String::new()
        } else {
            format!(" LIMIT {}", size + 1)
        };
        q.sql = format!(
            "SELECT {} AS value FROM {} r WHERE {} ORDER BY r.{} DESC,r.{} DESC{limit}",
            self.project_sql(e)?,
            quote(&e.table()),
            predicates.join(" AND "),
            col(e, "date_created")?,
            col(e, "id")?
        );
        let rows = self
            .db
            .rows(q)?
            .iter()
            .map(|raw| self.decode_row(e, raw))
            .collect::<Result<Vec<_>>>()?;
        if method == "Count" {
            let mut all = BTreeSet::new();
            let mut counts: BTreeMap<&str, (usize, BTreeSet<&str>)> = BTreeMap::new();
            for row in &rows {
                let entry = counts.entry(row["topicKey"].as_str().unwrap()).or_default();
                entry.0 += 1;
                if let Some(source) = row["sourceId"].as_str() {
                    all.insert(source);
                    entry.1.insert(source);
                }
            }
            let per = topics
                .into_iter()
                .map(|(key, topic)| {
                    let (n, s) = counts.remove(key.as_str()).unwrap_or_default();
                    json!({"topic":topic,"count":n.to_string(),"sourceCount":s.len().to_string()})
                })
                .collect::<Vec<_>>();
            return Ok(
                json!({"readerId":actor,"count":rows.len().to_string(),"sourceCount":all.len().to_string(),"topics":per}),
            );
        }
        let mut out = Vec::new();
        for row in rows.iter().take(size) {
            let n = notice(row)?;
            if method == "List" {
                out.push(n)
            } else {
                let mut item = json!({"notification":n});
                if let Some(v) = self
                    .matching(
                        receipts,
                        &[("notification_id", &row["id"]), ("reader_id", &actor)],
                        false,
                    )?
                    .first()
                {
                    item["receipt"] = receipt(v)
                }
                out.push(item);
            }
        }
        let mut result = json!({"items":out});
        if method == "Read" {
            result["readerId"] = actor;
        }
        if rows.len() > size {
            let row = &rows[size - 1];
            let bytes =
                json!({"scope":scope,"dateCreated":row["dateCreated"],"id":row["id"]}).to_string();
            let mut h = mac();
            h.update(bytes.as_bytes());
            result["next"] = json!(format!(
                "{}.{}",
                B64.encode(bytes),
                B64.encode(h.finalize().into_bytes())
            ));
        }
        Ok(result)
    }
}

impl crate::Store {
    pub fn notification_keep(&self) -> Result<chrono::Duration> {
        self.policies
            .values()
            .find(|p| p.entity["own"] == "OWN_HOLDER")
            .and_then(|p| p.application["notify"]["keep"]["days"].as_i64())
            .filter(|n| *n > 0)
            .and_then(chrono::Duration::try_days)
            .ok_or_else(|| Fault::precondition("positive notification retention days required"))
    }
    /// Erase one oldest-first batch. Each invocation commits its receipts,
    /// publication keys, audit rows and invalidations in one transaction.
    /// A zero tenant selects the account/global namespace. None visits all tenants.
    pub fn expire_notifications(
        &mut self,
        tenant: Option<[u8; 16]>,
        limit: usize,
    ) -> Result<usize> {
        if !(1..=10000).contains(&limit) {
            return Err(Fault::invalid("invalid expiry batch size"));
        }
        let keep = self.notification_keep()?;
        self.transaction(&Scope::Internal,|tx|{
            let app=&tx.policies.values().find(|p|p.entity["own"]=="OWN_HOLDER").ok_or_else(||Fault::precondition("missing application schema"))?.application;

            let before=(tx.inputs.clock)().checked_sub_signed(keep).ok_or_else(||Fault::invalid("retention exceeds timestamp range"))?;
            let ns=app["goPackage"].as_str().unwrap_or("").split(';').next().unwrap_or("");
            let n=tx.owned_entity("OWN_NOTIFICATION")?.clone();
            let r=tx.owned_entity("OWN_NOTIFICATION_RECEIPT")?.clone();
            let d=tx.db.dialect();
            let mut q=Query::new("");
            let time=q.param(Bind::Text(before.to_rfc3339_opts(chrono::SecondsFormat::Nanos,true)),d);
            let date=quote(&n.field("date_created").map_err(Fault::internal)?.column());
            let key=quote(&n.key().map_err(Fault::internal)?.column());
            let mut where_clause=format!("r.{date}<{time}");
            if let Some(t)=tenant{
                let arg=q.param(Bind::Bytes(t.to_vec()),d);
                where_clause.push_str(&format!(" AND r.{}={arg}",quote(&n.field("tenant_id").map_err(Fault::internal)?.column())));
            }
            let lock=if d==Dialect::Postgres{" FOR UPDATE"}else{""};
            q.sql=format!("SELECT {} AS value FROM {} r WHERE {where_clause} ORDER BY r.{date},r.{key} LIMIT {limit}{lock}",tx.project_sql(&n)?,quote(&n.table()));
            let rows=tx.db.rows(q)?.iter().map(|raw|tx.decode_row(&n,raw)).collect::<Result<Vec<_>>>()?;
            let grants=TopicGrants::default();
            for row in &rows{
                let tenant=value::bytes(&row["tenantId"])?;
                let ref_id=json!({"id":row["id"]});
                let frame=crate::entitle::deployment(tenant,"payday.notify.expire");
                let mut writer=Transaction{inputs:tx.inputs,resources:tx.resources,topic_policy:None,admission:None,origin:None,seen:None,changes:tx.changes,outbox:tx.outbox,namer:tx.namer,db:tx.db,model:tx.model,policies:tx.policies,scope:&Scope::Internal,frame:Some(&frame),failed:None};
                let mut del=Query::new("");
                let k=del.param(Bind::Bytes(value::bytes(&row["id"])?),d);
                del.sql=format!("DELETE FROM {} WHERE {}={k}",quote(&r.table()),quote(&r.field("notification_id").map_err(Fault::internal)?.column()));
                writer.db.exec(del)?;
                writer.erase(&n,&ref_id)?;
                let reader=Reader{namespace:ns,tenant:row["tenantId"].clone(),grants:&grants,actor:vec![]};
                writer.bump(&reader,row["topicKey"].as_str().ok_or_else(||Fault::internal("invalid stored topic"))?,"")?;
            }
            Ok(rows.len())
        })
    }
}
