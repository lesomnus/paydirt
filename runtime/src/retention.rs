//! Host-only trail lifecycle. SQLite serializes writers and PostgreSQL takes an
//! advisory transaction lock shared by policy updates, archival and reclamation.
use crate::{
    Fault, Result, Scope, Store, Transaction,
    archive_files::Archive,
    driver::{Bind, Query},
    value,
};
use chrono::{DateTime, TimeDelta, Utc};
use paydirt_model::{Dialect, quote};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keep {
    pub retain_ms: Option<u64>,
    pub destroy_ms: Option<u64>,
    pub discard: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub default: Keep,
    pub by: BTreeMap<String, Keep>,
    pub held: BTreeSet<String>,
}
#[derive(Serialize, Deserialize, Clone, Default)]
struct State {
    policy: Policy,
    retired: bool,
}
#[derive(Serialize, Deserialize, Clone)]
struct Entry {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manifest: Option<String>,
    tenant: String,
    kind: String,
    created: String,
    owners: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proof: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ciphertext: Option<String>,
    table: String,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signature: Option<crate::attestation::Signature>,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Policy {
        policy: Policy,
    },
    Inspect,
    Collect,
    Purge,
    PurgeKind {
        kind: String,
        before: DateTime<Utc>,
    },
    Retire,
    Forget {
        subject: String,
    },
    Retry,
    Export {
        recipient: String,
        operator: String,
        reason: String,
    },
}
#[derive(Deserialize)]
pub struct Request {
    pub tenant: String,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(flatten)]
    pub action: Action,
}
#[derive(Default, Serialize)]
pub struct Report {
    pub policy: Policy,
    pub inventory: BTreeMap<String, String>,
    pub archived: Vec<String>,
    pub keys: usize,
    pub carried: Vec<String>,
    pub removed: Vec<String>,
    pub held: Vec<String>,
    pub blanked: Vec<String>,
    pub owed: Vec<String>,
    pub exported: Vec<String>,
    pub bundle: Option<String>,
    pub retired: bool,
}
const DEPLOYMENT: &str = "00000000-0000-0000-0000-000000000000";
fn canonical_kind(policies: &crate::Policies, name: &str) -> Result<String> {
    let domains = crate::domain::Domains::from_policies(policies)?;
    let number = domains
        .lookup(name)
        .or_else(|| domains.domain_of(name))
        .ok_or_else(|| Fault::invalid(format!("unknown or ambiguous trail kind {name}")))?;
    let mut matches = policies
        .iter()
        .filter(|(_, p)| p.entity["domain"].as_u64() == Some(number as u64));
    let (entity, policy) = matches
        .next()
        .ok_or_else(|| Fault::invalid("unknown domain"))?;
    if matches.next().is_some() {
        return Err(Fault::invalid(format!("ambiguous trail kind {name}")));
    }
    Ok(if policy.entity["own"] == "OWN_ACCESS" {
        "access".into()
    } else {
        entity.clone()
    })
}
impl Policy {
    fn resolve(&self, policies: &crate::Policies) -> Result<Self> {
        let mut by = BTreeMap::new();
        for (name, value) in &self.by {
            if by
                .insert(canonical_kind(policies, name)?, value.clone())
                .is_some()
            {
                return Err(Fault::invalid("trail policy names the same kind twice"));
            }
        }
        let mut held = BTreeSet::new();
        for name in &self.held {
            let kind = if name == "*" {
                name.clone()
            } else {
                canonical_kind(policies, name)?
            };
            if !held.insert(kind) {
                return Err(Fault::invalid("trail hold names the same kind twice"));
            }
        }
        Ok(Self {
            default: self.default.clone(),
            by,
            held,
        })
    }
    fn keep(&self, kind: &str) -> &Keep {
        self.by.get(kind).unwrap_or(&self.default)
    }
}

fn decode<T: DeserializeOwned>(v: &str) -> Result<T> {
    serde_json::from_str(v).map_err(|e| Fault::precondition(e.to_string()))
}
fn bad(e: impl ToString) -> Fault {
    Fault::precondition(e.to_string())
}
fn uuid(s: &str) -> Result<String> {
    paydirt::parse_id(s)
        .filter(|i| *i != [0; 16])
        .and_then(|i| paydirt::format_id(&i))
        .ok_or_else(|| Fault::invalid("invalid UUID"))
}
fn row_id(v: &Value) -> Option<String> {
    paydirt::format_id(&value::bytes(v).ok()?)
        .filter(|s| s != "00000000-0000-0000-0000-000000000000")
}
fn param(q: &mut Query, s: &str, d: Dialect) -> String {
    q.param(Bind::Text(s.into()), d)
}
fn due(start: &str, ms: Option<u64>, now: DateTime<Utc>) -> Result<bool> {
    let Some(ms) = ms else { return Ok(false) };
    let age = TimeDelta::try_milliseconds(ms.try_into().map_err(bad)?)
        .ok_or_else(|| Fault::invalid("invalid retention duration"))?;
    Ok(start
        .parse::<DateTime<Utc>>()
        .map_err(bad)?
        .checked_add_signed(age)
        .is_some_and(|v| v <= now))
}
const TABLES: &str =
    "CREATE TABLE IF NOT EXISTS paydirt_trail_policy (id TEXT PRIMARY KEY, doc TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS paydirt_trail_archive (id TEXT PRIMARY KEY, doc TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS paydirt_trail_owed (id TEXT PRIMARY KEY, doc TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS paydirt_trail_removed (id TEXT PRIMARY KEY, doc TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS paydirt_trail_gc (id TEXT PRIMARY KEY, doc TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS paydirt_trail_receipt (id TEXT PRIMARY KEY, doc TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS paydirt_trail_format (id TEXT PRIMARY KEY, doc TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS paydirt_checkpoint_pending (id TEXT PRIMARY KEY, doc TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS paydirt_trail_key_gc (id TEXT PRIMARY KEY, doc TEXT NOT NULL);";
#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum Obligation {
    Current {
        tenant: String,
        subject: String,
        key: String,
    },
    Legacy((String, String)),
}
impl Obligation {
    fn subject(&self) -> &str {
        match self {
            Self::Current { subject, .. } => subject,
            Self::Legacy((_, subject)) => subject,
        }
    }
}
impl Transaction<'_> {
    fn lock_trail(&mut self) -> Result<()> {
        if self.db.dialect() == Dialect::Postgres {
            self.db.rows(Query::new(
                "SELECT '1' AS value FROM pg_advisory_xact_lock(102774192,51)",
            ))?;
        }
        self.db.batch(TABLES)?;
        if self.db.dialect() == Dialect::Postgres {
            self.db.batch(r#"CREATE INDEX IF NOT EXISTS paydirt_trail_owed_seek ON paydirt_trail_owed (id COLLATE "C")"#)?;
        }
        Ok(())
    }
    /// Queue a subject erasure in the same transaction as the host's business write.
    /// The maintenance pass services the obligation after commit and respects holds.
    pub fn owe_trail_erasure(&mut self, tenant: [u8; 16], subject: [u8; 16]) -> Result<()> {
        if !matches!(self.scope, Scope::Internal) {
            return Err(Fault::denied(
                "erasure obligations require a host transaction",
            ));
        }
        if !paydirt::has_domain(&tenant, 1) || subject == [0; 16] {
            return Err(Fault::invalid("expected tenant and subject IDs"));
        }
        let tenant = paydirt::format_id(&tenant).expect("sixteen-byte identifier");
        let subject = paydirt::format_id(&subject).expect("sixteen-byte identifier");
        self.lock_trail()?;
        self.queue_obligation(&tenant, &subject).map(|_| ())
    }
    /// Blanks online audit content in the caller's transaction and leaves the
    /// independent archive work queued until that transaction commits.
    pub fn forget_trail_in(&mut self, tenant: [u8; 16], subjects: &[[u8; 16]]) -> Result<Report> {
        if !matches!(self.scope, Scope::Internal) {
            return Err(Fault::denied("erasure requires a host transaction"));
        }
        if !paydirt::has_domain(&tenant, 1) || subjects.contains(&[0; 16]) {
            return Err(Fault::invalid("expected tenant and subject IDs"));
        }
        let mut report = Report::default();
        if subjects.is_empty() {
            return Ok(report);
        }
        self.lock_trail()?;
        let states: BTreeMap<String, State> = self
            .trail_docs::<(String, State)>("paydirt_trail_policy")?
            .into_iter()
            .collect();
        let default = states.get(DEPLOYMENT).cloned().unwrap_or_default();
        let subjects: BTreeSet<_> = subjects.iter().copied().collect();
        let tenant = paydirt::format_id(&tenant).unwrap();
        // Bound each IN list for deployments using the SQLite variable limit.
        for chunk in subjects.iter().copied().collect::<Vec<_>>().chunks(200) {
            let select = crate::trail_page::Selection {
                whose: crate::trail_page::Whose::Named,
                tenant: paydirt::parse_id(&tenant).unwrap(),
                objects: chunk.iter().copied().collect(),
                ..Default::default()
            };
            for (entry, row) in self.selected_live_trail(&select)? {
                if !row["objectId"].is_string() {
                    continue;
                }
                let held = entry.owners.iter().chain([&entry.tenant]).any(|owner| {
                    let p = &states.get(owner).unwrap_or(&default).policy;
                    p.held.contains("*") || p.held.contains(&entry.kind)
                });
                if held {
                    report.held.push(entry.id);
                    continue;
                }
                if ["value", "patch"]
                    .iter()
                    .any(|k| row[k].as_str().is_some_and(|s| !s.is_empty()))
                {
                    self.alter_live(&entry, true)?;
                    report.blanked.push(entry.id);
                }
            }
        }
        for subject in subjects {
            let subject = paydirt::format_id(&subject).unwrap();
            self.queue_obligation(&tenant, &subject)?;
            report.owed.push(subject);
        }
        Ok(report)
    }
    fn queue_obligation(&mut self, tenant: &str, subject: &str) -> Result<Obligation> {
        let id = format!("{tenant}/{subject}");
        let mut q = Query::new("");
        let bound = param(&mut q, &id, self.db.dialect());
        q.sql = format!("SELECT doc AS value FROM paydirt_trail_owed WHERE id={bound}");
        if let Some(doc) = self.db.rows(q)?.first()
            && let obligation @ Obligation::Current { .. } = decode(doc)?
        {
            return Ok(obligation);
        }
        let obligation = if let Some((name, p)) = self
            .policies
            .iter()
            .find(|(_, p)| p.entity["own"] == "OWN_OWED")
        {
            let domain = p.entity["domain"]
                .as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .ok_or_else(|| bad("missing erasure queue domain"))?;
            let key = (self.inputs.mint)(domain);
            let e = self.model.entity(name).map_err(bad)?;
            self.insert(e,&json!({"id":value::encoded(&key),"object":value::encoded(&paydirt::parse_id(subject).ok_or_else(||bad("invalid subject"))?),"heldBy":[]}))?;
            Obligation::Current {
                tenant: tenant.into(),
                subject: subject.into(),
                key: paydirt::format_id(&key).unwrap(),
            }
        } else {
            Obligation::Legacy((tenant.into(), subject.into()))
        };
        self.trail_set("paydirt_trail_owed", &id, Some(&obligation))?;
        Ok(obligation)
    }
    fn obligations(&mut self, tenant: &str, upgrade: bool) -> Result<BTreeMap<String, Obligation>> {
        let mut out = BTreeMap::new();
        let end = format!("{tenant}0");
        let mut cursor = format!("{tenant}/");
        loop {
            let mut q = Query::new("");
            let d = self.db.dialect();
            let start = param(&mut q, &cursor, d);
            let end = param(&mut q, &end, d);
            let projection = match d {
                Dialect::Postgres => "json_build_array(id,doc)::text",
                Dialect::Sqlite => "json_array(id,doc)",
            };
            let id = if d == Dialect::Postgres {
                r#"id COLLATE "C""#
            } else {
                "id COLLATE BINARY"
            };
            q.sql = format!(
                "SELECT {projection} AS value FROM paydirt_trail_owed WHERE {id}>{start} AND {id}<{end} ORDER BY {id} LIMIT 256"
            );
            let page = self.db.rows(q)?;
            let count = page.len();
            for raw in page {
                let (key, doc): (String, String) = decode(&raw)?;
                let mut entry: Obligation = decode(&doc)?;
                let subject = uuid(entry.subject())?;
                if key != format!("{tenant}/{subject}") {
                    return Err(bad("erasure obligation key mismatch"));
                }
                cursor = key;
                if upgrade && matches!(entry, Obligation::Legacy(_)) {
                    entry = self.queue_obligation(tenant, &subject)?;
                }
                out.insert(subject, entry);
            }
            if count < 256 {
                break;
            }
        }
        Ok(out)
    }
    fn finish_obligation(
        &mut self,
        tenant: &str,
        obligation: &Obligation,
        held_by: &BTreeSet<String>,
    ) -> Result<()> {
        if let Obligation::Current { key, .. } = obligation {
            let (name, _) = self
                .policies
                .iter()
                .find(|(_, p)| p.entity["own"] == "OWN_OWED")
                .ok_or_else(|| bad("erasure queue schema missing"))?;
            let e = self.model.entity(name).map_err(bad)?;
            let d = self.db.dialect();
            let mut q = Query::new("");
            let bound = q.param(
                Bind::Bytes(
                    paydirt::parse_id(key)
                        .ok_or_else(|| bad("invalid erasure queue id"))?
                        .to_vec(),
                ),
                d,
            );
            let table = quote(&e.table());
            let key_col = quote(&e.key().map_err(bad)?.column());
            if held_by.is_empty() {
                q.sql = format!("DELETE FROM {table} WHERE {key_col}={bound}");
            } else {
                let held: Vec<_> = held_by
                    .iter()
                    .map(|v| {
                        paydirt::parse_id(v)
                            .map(|id| value::encoded(&id))
                            .ok_or_else(|| bad("invalid hold tenant"))
                    })
                    .collect::<Result<_>>()?;
                let field = e.field("held_by").map_err(bad)?;
                let owners = q.param(value::bind(field, &json!(held))?, d);
                q.sql = format!(
                    "UPDATE {table} SET {}={owners} WHERE {key_col}={bound}",
                    quote(&field.column())
                );
            }
            self.db.exec(q)?;
        }
        if held_by.is_empty() {
            self.trail_clear(
                "paydirt_trail_owed",
                &format!("{tenant}/{}", obligation.subject()),
            )?;
        }
        Ok(())
    }
    fn trail_docs<T: DeserializeOwned>(&mut self, table: &str) -> Result<Vec<T>> {
        self.db
            .rows(Query::new(format!(
                "SELECT doc AS value FROM {table} ORDER BY id"
            )))?
            .into_iter()
            .map(|s| decode(&s))
            .collect()
    }
    fn trail_set(&mut self, table: &str, id: &str, doc: Option<&impl Serialize>) -> Result<()> {
        let mut q = Query::new("");
        let d = self.db.dialect();
        let i = param(&mut q, id, d);
        q.sql = if let Some(v) = doc {
            let v = param(&mut q, &serde_json::to_string(v).map_err(bad)?, d);
            format!(
                "INSERT INTO {table} (id,doc) VALUES ({i},{v}) ON CONFLICT(id) DO UPDATE SET doc=excluded.doc"
            )
        } else {
            format!("DELETE FROM {table} WHERE id={i}")
        };
        self.db.exec(q)?;
        Ok(())
    }
    fn trail_clear(&mut self, table: &str, id: &str) -> Result<()> {
        self.trail_set(table, id, None::<&Value>)
    }
    fn live_trail(&mut self, tenant: &str, shared: bool) -> Result<Vec<(Entry, Value)>> {
        use crate::trail_page::{Selection, Whose};
        let select = Selection {
            whose: if shared {
                Whose::Named
            } else {
                Whose::FiledUnder
            },
            tenant: paydirt::parse_id(tenant).ok_or_else(|| bad("invalid tenant"))?,
            ..Default::default()
        };
        self.selected_live_trail(&select)
    }
    fn selected_live_trail(
        &mut self,
        select: &crate::trail_page::Selection,
    ) -> Result<Vec<(Entry, Value)>> {
        let mut rows = Vec::new();
        let mut after = None;
        loop {
            let page = self.read_trail_page(select, None, after, 256, false, true)?;
            let count = page.len();
            for item in page {
                after = Some(item.cursor);
                let row = item.document;
                let kind = if self.policies[&item.entity].entity["own"] == "OWN_ACCESS" {
                    "access".into()
                } else {
                    self.policies
                        .iter()
                        .find(|(_, p)| p.entity["domain"] == row["domain"])
                        .map(|(n, _)| n.clone())
                        .unwrap_or_else(|| format!("domain.{}", row["domain"]))
                };
                let owners = item
                    .tenants
                    .iter()
                    .filter_map(|id| paydirt::format_id(id))
                    .collect();
                rows.push((
                    Entry {
                        id: paydirt::format_id(&item.cursor.key).unwrap(),
                        tenant: row_id(&row["tenantId"])
                            .unwrap_or_else(|| "00000000-0000-0000-0000-000000000000".into()),
                        kind,
                        created: row["dateCreated"]
                            .as_str()
                            .ok_or_else(|| bad("missing trail time"))?
                            .into(),
                        owners,
                        namespace: None,
                        manifest: None,
                        proof: None,
                        ciphertext: None,
                        table: item.entity,
                        digest: None,
                        signature: None,
                    },
                    row,
                ));
            }
            if count < 256 {
                break;
            }
        }
        Ok(rows)
    }
    fn alter_live(&mut self, e: &Entry, blank: bool) -> Result<()> {
        let entity = self.model.entity(&e.table).map_err(bad)?;
        let mut q = Query::new("");
        let id = q.param(
            Bind::Bytes(
                paydirt::parse_id(&e.id)
                    .ok_or_else(|| bad("invalid manifest row ID"))?
                    .to_vec(),
            ),
            self.db.dialect(),
        );
        let body = if blank {
            let empty = q.param(Bind::Bytes(vec![]), self.db.dialect());
            let fields: Vec<_> = ["patch", "value"]
                .iter()
                .filter_map(|n| entity.field(n).ok())
                .map(|f| format!("{}={empty}", quote(&f.column())))
                .collect();
            format!("UPDATE {} SET {}", quote(&entity.table()), fields.join(","))
        } else {
            format!("DELETE FROM {}", quote(&entity.table()))
        };
        q.sql = format!(
            "{body} WHERE {}={id}",
            quote(&entity.key().map_err(bad)?.column())
        );
        self.db.exec(q)?;
        Ok(())
    }
    fn store_blob(&mut self, files: &Archive, e: &mut Entry, row: &Value) -> Result<()> {
        if e.digest.is_some() {
            self.removal(files, e)?;
        }
        if e.digest.is_none() && e.owners.len() > 1 {
            e.namespace = Some("shared".into());
        }
        e.manifest = None;
        self.begin_blob(files, e)?;
        let stored = if let Some(keys) = &files.tenant_keys {
            keys.seal(row.clone(), &e.owners, e.created.parse().map_err(bad)?)?
        } else {
            row.clone()
        };
        let hash = files.stage(&serde_json::to_vec(&stored).map_err(bad)?)?;
        if let Some(old) = e.digest.replace(hash) {
            self.trail_set("paydirt_trail_gc", &old, Some(&old))?;
        }
        if files.notary.is_some() && row.get("objectId").is_some() {
            e.proof = Some(evidence(row));
            e.ciphertext =
                Some(files.ciphertext_digest(e.digest.as_deref().expect("staged digest"))?);
        }
        self.settle_blob(
            files,
            e,
            crate::manifest::State::Adding,
            crate::manifest::State::Present,
        )?;
        e.sign(files, "present")?;
        self.trail_set("paydirt_trail_archive", &e.id, Some(e))?;
        Ok(())
    }
    fn collect_keys(&mut self, files: &Archive) -> Result<()> {
        let owed = self.trail_docs::<(String, DateTime<Utc>)>("paydirt_trail_key_gc")?;
        for (owner, at) in owed {
            let keys = files
                .tenant_keys
                .as_ref()
                .ok_or_else(|| bad("pending destruction requires tenant key store"))?;
            keys.destroy(&owner, at)?;
            self.trail_clear("paydirt_trail_key_gc", &owner)?;
        }
        Ok(())
    }
    fn collect_gc(&mut self, files: &Archive) -> Result<()> {
        self.collect_keys(files)?;
        for item in self.trail_docs::<SignedReceipt>("paydirt_trail_receipt")? {
            files.check_receipt(&item)?;
            let id = uuid(&item.receipt.id)?;
            crate::attestation::write_once(
                &files
                    .root
                    .join(".maintenance-receipts")
                    .join(format!("{id}.json")),
                &serde_json::to_vec(&item).map_err(bad)?,
            )?;
            self.trail_clear("paydirt_trail_receipt", &id)?;
        }
        let active: BTreeSet<String> = self
            .trail_docs::<Entry>("paydirt_trail_archive")?
            .into_iter()
            .filter_map(|e| e.digest)
            .collect();
        files.recover(&active)?;
        for digest in self.trail_docs::<String>("paydirt_trail_gc")? {
            if !active.contains(&digest) {
                files.remove(&digest)?;
            }
            self.trail_clear("paydirt_trail_gc", &digest)?;
        }
        Ok(())
    }
}
impl Store {
    pub fn manage_trail(
        &mut self,
        files: &Archive,
        recipient: Option<&Archive>,
        request: Request,
    ) -> Result<Report> {
        let tenant = if request.tenant == "deployment" {
            if !matches!(
                request.action,
                Action::Policy { .. } | Action::Inspect | Action::Collect
            ) {
                return Err(Fault::invalid("expected a tenant ID for this action"));
            }
            DEPLOYMENT.to_owned()
        } else {
            let tenant = uuid(&request.tenant)?;
            if !paydirt::has_domain(&paydirt::parse_id(&tenant).unwrap(), 1) {
                return Err(Fault::invalid("not a tenant ID"));
            }
            tenant
        };
        self.manage_filing(files, recipient, request, tenant, false)
    }
    /// Applies the deployment policy and per-tenant overrides, once per filing tenant.
    pub fn sweep_trail(&mut self, files: &Archive, dry: bool) -> Result<BTreeMap<String, Report>> {
        let tenants = self.transaction(&Scope::Internal, |tx| {
            tx.lock_trail()?;
            let mut tenants = BTreeSet::from([DEPLOYMENT.to_owned()]);
            let mut after = [0; 16];
            loop {
                let page = tx.trail_tenants(after, 256)?;
                for id in &page {
                    tenants.insert(paydirt::format_id(id).unwrap());
                }
                if page.len() < 256 {
                    break;
                }
                after = *page.last().unwrap();
            }
            tenants.extend(
                tx.trail_docs::<Entry>("paydirt_trail_archive")?
                    .into_iter()
                    .map(|e| e.tenant),
            );
            tenants.extend(
                tx.trail_docs::<(String, State)>("paydirt_trail_policy")?
                    .into_iter()
                    .map(|v| v.0),
            );
            for e in tx.trail_docs::<Obligation>("paydirt_trail_owed")? {
                tenants.insert(match e {
                    Obligation::Current { tenant, .. } | Obligation::Legacy((tenant, _)) => tenant,
                });
            }
            Ok(tenants)
        })?;
        let mut reports = BTreeMap::new();
        for tenant in tenants {
            let req = Request {
                tenant: tenant.clone(),
                dry_run: dry,
                action: Action::Collect,
            };
            let result = self.manage_filing(files, None, req, tenant.clone(), true)?;
            reports.insert(tenant, result);
        }
        Ok(reports)
    }
    fn manage_filing(
        &mut self,
        files: &Archive,
        recipient: Option<&Archive>,
        request: Request,
        tenant: String,
        filed_only: bool,
    ) -> Result<Report> {
        let dry = request.dry_run;
        let maintenance = !dry
            && files.notary.is_some()
            && matches!(
                request.action,
                Action::Collect
                    | Action::Purge
                    | Action::PurgeKind { .. }
                    | Action::Retire
                    | Action::Forget { .. }
                    | Action::Retry
            );
        let (out, publish) = self.transaction(&Scope::Internal, |tx| {
            tx.lock_trail()?;
            if !dry
                && !matches!(
                    request.action,
                    Action::Inspect | Action::Export { .. } | Action::Policy { .. }
                )
            {
                tx.collect_keys(files)?;
            }
            if matches!(request.action, Action::Purge) && files.tenant_keys.is_some() {
                let preview = run_pass(
                    tx,
                    files,
                    recipient,
                    &request,
                    tenant.clone(),
                    true,
                    filed_only,
                )?;
                if !preview.held.is_empty() || preview.policy.held.contains("*") {
                    return Err(Fault::precondition("tenant archive is under a hold"));
                }
            }
            if maintenance && tx.pending_checkpoint()?.is_some() {
                tx.checkpoint(files, true, None)?;
            }
            let report = run_pass(tx, files, recipient, &request, tenant, dry, filed_only)?;
            let publish = maintenance
                && (!report.archived.is_empty()
                    || !report.removed.is_empty()
                    || !report.blanked.is_empty());
            if publish {
                tx.prepare_checkpoint(files)?;
            }
            Ok((report, publish))
        })?;
        if publish {
            self.attest_trail(files, true)?;
        }
        if !dry
            && !matches!(
                request.action,
                Action::Inspect | Action::Export { .. } | Action::Policy { .. }
            )
        {
            self.transaction(&Scope::Internal, |tx| {
                tx.lock_trail()?;
                tx.collect_gc(files)
            })?;
        }
        Ok(out)
    }
}

fn run_pass(
    tx: &mut Transaction<'_>,
    files: &Archive,
    recipient: Option<&Archive>,
    request: &Request,
    tenant: String,
    dry: bool,
    filed_only: bool,
) -> Result<Report> {
    tx.lock_trail()?;
    let now = (tx.inputs.clock)();
    let mut states: BTreeMap<String, State> = tx
        .trail_docs::<(String, State)>("paydirt_trail_policy")?
        .into_iter()
        .collect();
    let deployment = states.get(DEPLOYMENT).cloned().unwrap_or_default();
    let mut state = states
        .get(&tenant)
        .cloned()
        .unwrap_or_else(|| deployment.clone());
    if matches!(request.action, Action::Purge) && state.policy.held.contains("*") {
        return Err(Fault::precondition("tenant archive is under a hold"));
    }
    let purge_kind = match &request.action {
        Action::PurgeKind { kind, before } => Some((canonical_kind(tx.policies, kind)?, *before)),
        _ => None,
    };
    let mut counts: BTreeMap<(&str, &str, String), usize> = BTreeMap::new();
    if let Action::Policy { policy } = &request.action {
        for k in policy.by.values().chain([&policy.default]) {
            if matches!((k.retain_ms,k.destroy_ms),(Some(r),Some(d))if d<r) {
                return Err(Fault::invalid("destroy window precedes retention"));
            }
        }
        state.policy = policy.resolve(tx.policies)?;
        if !dry {
            tx.trail_set("paydirt_trail_policy", &tenant, Some(&(&tenant, &state)))?;
        }
        return Ok(Report {
            policy: state.policy.clone(),
            retired: state.retired,
            ..Default::default()
        });
    }
    if matches!(&request.action, Action::Retire) {
        state.retired = true;
        if !dry {
            tx.trail_set("paydirt_trail_policy", &tenant, Some(&(&tenant, &state)))?;
        }
    }
    states.insert(tenant.clone(), state.clone());
    let mut out = Report {
        policy: state.policy.clone(),
        retired: state.retired,
        ..Default::default()
    };
    let mut obligations = tx.obligations(&tenant, !dry)?;
    if let Action::Forget { subject } = &request.action {
        let subject = uuid(subject)?;
        let item = if dry {
            Obligation::Legacy((tenant.clone(), subject.clone()))
        } else {
            tx.queue_obligation(&tenant, &subject)?
        };
        obligations.insert(subject, item);
    }
    let owed: BTreeSet<_> = obligations.keys().cloned().collect();
    let mut waiting: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut exported = Vec::new();
    let mut carry = crate::integrity::CarryHeads::default();
    let shared = (!filed_only || !owed.is_empty())
        && tenant != DEPLOYMENT
        && !matches!(
            request.action,
            Action::Retire | Action::Purge | Action::PurgeKind { .. }
        );
    let mut rows = tx.live_trail(&tenant, shared)?;
    for entry in tx.trail_docs::<Entry>("paydirt_trail_archive")?.into_iter() {
        tx.check_archive_entry(files, &entry, "present")?;
        if entry.tenant != tenant && !(shared && entry.owners.contains(&tenant)) {
            continue;
        }
        let digest = entry
            .digest
            .as_deref()
            .ok_or_else(|| bad("missing archive digest"))?;
        let row = serde_json::from_slice(&files.read(digest)?).map_err(bad)?;
        rows.push((entry, row));
    }
    rows.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    for (mut e, mut row) in rows {
        if matches!(request.action, Action::Inspect) {
            out.inventory.insert(
                e.id.clone(),
                if e.digest.is_some() {
                    "archived".into()
                } else {
                    "online".into()
                },
            );
            continue;
        }
        let owners: BTreeSet<_> = e.owners.iter().chain([&e.tenant]).collect();
        let policy = |owner: &str| &states.get(owner).unwrap_or(&deployment).policy;
        let is_held =
            |owner: &str| policy(owner).held.contains("*") || policy(owner).held.contains(&e.kind);
        let held = owners.iter().any(|t| is_held(t));
        let mut collect = true;
        let mut destroy = true;
        let mut discard = true;
        for owner in &owners {
            let keep = policy(owner).keep(&e.kind);
            discard &= keep.discard;
            let cold = due(&e.created, keep.retain_ms, now)?;
            collect &= cold;
            destroy &= due(&e.created, keep.destroy_ms, now)? || keep.discard && cold;
        }
        let k = state.policy.keep(&e.kind);
        let archived = e.digest.is_some();
        let forget = matches!(
            request.action,
            Action::Forget { .. } | Action::Retry | Action::Collect
        );
        let subjects: BTreeSet<_> = row["subjects"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(row_id)
            .chain(row_id(&row["objectId"]))
            .filter(|s| owed.contains(s))
            .collect();
        let selected = if let Some((kind, before)) = &purge_kind {
            archived
                && e.kind == *kind
                && e.created.parse::<DateTime<Utc>>().map_err(bad)? < *before
        } else {
            false
        };
        let filing = !filed_only || e.tenant == tenant;
        let target = selected
            || matches!(request.action, Action::Purge)
            || filing
                && matches!(request.action, Action::Collect | Action::Retire)
                && (destroy || state.retired && k.retain_ms.is_none() && k.destroy_ms.is_none());
        if matches!(request.action, Action::Export { .. }) {
            out.exported.push(e.id.clone());
            exported.push(files.open_trail_row(row, &e.owners)?);
            continue;
        }
        let shared_departure = e.owners.len() > 1
            && (matches!(request.action, Action::Purge | Action::Retire)
                || state.retired && e.tenant == tenant);
        if held && (target || shared_departure || forget && !subjects.is_empty()) {
            out.held.push(e.id.clone());
            for subject in &subjects {
                let owners = waiting.entry(subject.clone()).or_default();
                owners.extend(
                    e.owners
                        .iter()
                        .chain([&e.tenant])
                        .filter(|t| is_held(t))
                        .cloned(),
                );
            }
        }
        if target && !held && !shared_departure {
            if matches!(request.action, Action::Collect)
                && !state.retired
                && !row["objectId"].is_null()
                && tx.carry_latest(&e.kind, &row, files, &mut carry, dry)?
            {
                out.carried
                    .push(row_id(&row["objectId"]).ok_or_else(|| bad("invalid carried id"))?);
            }
            let (act, place) = match &request.action {
                Action::PurgeKind { .. } => ("purge", "archive"),
                Action::Purge | Action::Retire => (
                    "purge-tenant",
                    if archived { "archive" } else { "database" },
                ),
                _ if !archived && discard => ("discard", "database"),
                _ => ("destroy", "archive"),
            };
            *counts.entry((act, place, e.kind.clone())).or_default() += 1;
            if act == "destroy" && !archived && !dry {
                tx.store_blob(files, &mut e, &row)?;
                tx.alter_live(&e, false)?;
            }
            out.removed.push(e.id.clone());
            if !dry {
                if let Some(old) = &e.digest {
                    tx.removal(files, &e)?;
                    tx.trail_clear("paydirt_trail_archive", &e.id)?;
                    tx.trail_set("paydirt_trail_gc", old, Some(old))?;
                } else {
                    tx.alter_live(&e, false)?;
                }
            }
            continue;
        }
        let blank = !held
            && (shared_departure || forget && !subjects.is_empty())
            && ["patch", "value", "sealed"]
                .iter()
                .any(|k| row[k].as_str().is_some_and(|s| !s.is_empty()));
        if blank {
            *counts
                .entry((
                    "forget",
                    if archived { "archive" } else { "database" },
                    e.kind.clone(),
                ))
                .or_default() += 1;
            out.blanked.push(e.id.clone());
            row.as_object_mut()
                .ok_or_else(|| bad("invalid trail row"))?
                .remove("sealed");
            for key in ["patch", "value"] {
                if row.get(key).is_some() {
                    row[key] = json!("")
                }
            }
        }
        let transfer = filing
            && !archived
            && matches!(
                request.action,
                Action::Collect | Action::Retire | Action::Purge
            )
            && (collect || state.retired || target && held);
        if transfer {
            out.archived.push(e.id.clone());
        }
        if !dry && (transfer || archived && blank) {
            tx.store_blob(files, &mut e, &row)?;
            if transfer {
                tx.alter_live(&e, false)?;
            }
        } else if !dry && blank {
            tx.alter_live(&e, true)?;
        }
    }
    if let Action::Export {
        recipient: name,
        operator,
        reason,
    } = &request.action
    {
        if [name, operator, reason]
            .into_iter()
            .any(|s| s.trim().is_empty())
        {
            return Err(Fault::invalid(
                "export needs a recipient, operator and reason",
            ));
        }
        if !dry {
            let bundle = json!({"format":1,"tenant":tenant,"recipient":name,"operator":operator,"reason":reason,"rows":exported});
            out.bundle = Some(
                recipient
                    .ok_or_else(|| Fault::invalid("missing recipient archive key"))?
                    .put(&serde_json::to_vec(&bundle).map_err(bad)?)?,
            );
        }
    }
    if matches!(
        request.action,
        Action::Forget { .. } | Action::Retry | Action::Collect
    ) {
        if !dry {
            let empty = BTreeSet::new();
            for (subject, obligation) in &obligations {
                let holds = waiting.get(subject).unwrap_or(&empty);
                if holds.is_empty() {
                    tx.forget_seal(
                        &paydirt::parse_id(subject).ok_or_else(|| bad("invalid subject"))?,
                    )?;
                }
                tx.finish_obligation(&tenant, obligation, holds)?;
            }
        }
        out.owed = waiting.into_keys().collect();
    } else {
        out.owed = owed.into_iter().collect();
    }
    if matches!(request.action, Action::Purge)
        && let Some(keys) = &files.tenant_keys
    {
        out.keys = usize::from(keys.has_key(&tenant)?);
        if !dry {
            tx.trail_set("paydirt_trail_key_gc", &tenant, Some(&(&tenant, now)))?;
        }
    }
    if !dry {
        for ((act, place, kind), rows) in counts {
            let id = paydirt::format_id(&paydirt::mint(0)).unwrap();
            let receipt = Receipt {
                id: id.clone(),
                at: now,
                tenant: tenant.clone(),
                kind,
                act: act.into(),
                place: place.into(),
                rows,
            };
            let signature = files
                .notary
                .as_ref()
                .map(|n| n.sign(&receipt.statement()?))
                .transpose()?;
            tx.trail_set(
                "paydirt_trail_receipt",
                &id,
                Some(&SignedReceipt { receipt, signature }),
            )?;
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub id: String,
    pub at: DateTime<Utc>,
    pub tenant: String,
    pub kind: String,
    pub act: String,
    #[serde(rename = "where")]
    pub place: String,
    pub rows: usize,
}
impl Receipt {
    fn statement(&self) -> Result<Vec<u8>> {
        let mut out = b"paydirt.trail.receipt.v1\0".to_vec();
        out.extend(serde_json::to_vec(self).map_err(bad)?);
        Ok(out)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedReceipt {
    receipt: Receipt,
    signature: Option<crate::attestation::Signature>,
}
impl Archive {
    fn check_receipt(&self, item: &SignedReceipt) -> Result<()> {
        match (&self.notary, &item.signature) {
            (Some(n), Some(sig)) => n.verify(&item.receipt.statement()?, sig),
            (None, None) => Ok(()),
            _ => Err(bad("receipt signature configuration mismatch")),
        }
    }
    pub fn trail_receipts(&self) -> Result<Vec<Receipt>> {
        let dir = self.root.join(".maintenance-receipts");
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(bad(e)),
        };
        let mut rows = Vec::new();
        for file in entries {
            let path = file.map_err(bad)?.path();
            if path.extension().and_then(|v| v.to_str()) != Some("json") {
                continue;
            }
            let item: SignedReceipt =
                serde_json::from_slice(&std::fs::read(&path).map_err(bad)?).map_err(bad)?;
            self.check_receipt(&item)?;
            if path.file_stem().and_then(|v| v.to_str()) != Some(item.receipt.id.as_str()) {
                return Err(bad("receipt filename mismatch"));
            }
            rows.push(item.receipt);
        }
        rows.sort_by(|a, b| (a.at, &a.id).cmp(&(b.at, &b.id)));
        Ok(rows)
    }
}
fn evidence(row: &Value) -> Value {
    let selected = [
        "id",
        "objectId",
        "tenantId",
        "dateCreated",
        "domain",
        "action",
        "digest",
        "digestFields",
    ]
    .into_iter()
    .filter_map(|name| row.get(name).map(|v| (name.into(), v.clone())))
    .collect();
    Value::Object(selected)
}
impl Archive {
    fn open_trail_row(&self, row: Value, owners: &BTreeSet<String>) -> Result<Value> {
        if row.get("sealed").is_none() {
            return Ok(row);
        }
        self.tenant_keys
            .as_ref()
            .ok_or_else(|| bad("archived contents are sealed; a master identity is required"))?
            .open_row(row, owners)
    }
}
impl Store {
    /// Returns online and archived records visible to the tenant, in creation order.
    pub fn read_tenant_trail(&mut self, files: &Archive, tenant: [u8; 16]) -> Result<Vec<Value>> {
        if !paydirt::has_domain(&tenant, 1) {
            return Err(Fault::invalid("expected a tenant identifier"));
        }
        let tenant = paydirt::format_id(&tenant).unwrap();
        self.transaction(&Scope::Internal, |tx| {
            tx.lock_trail()?;
            let mut rows = tx
                .live_trail(&tenant, true)?
                .into_iter()
                .map(|(_, r)| r)
                .collect::<Vec<_>>();
            for e in tx.trail_docs::<Entry>("paydirt_trail_archive")? {
                if !e.owners.contains(&tenant) {
                    continue;
                }
                tx.check_archive_entry(files, &e, "present")?;
                let data = files.read(
                    e.digest
                        .as_deref()
                        .ok_or_else(|| bad("missing archive digest"))?,
                )?;
                rows.push(
                    files.open_trail_row(serde_json::from_slice(&data).map_err(bad)?, &e.owners)?,
                );
            }
            let mut ordered = rows
                .into_iter()
                .map(|v| Ok((crate::integrity::order(&v)?, v)))
                .collect::<Result<Vec<_>>>()?;
            ordered.sort_by_key(|v| v.0);
            Ok(ordered.into_iter().map(|v| v.1).collect())
        })
    }
}
impl Entry {
    fn blob(&self) -> Result<crate::attestation::Blob> {
        self.digest
            .as_ref()
            .map(|d| {
                crate::attestation::Blob::of(self.namespace.as_deref().unwrap_or(&self.tenant), d)
            })
            .ok_or_else(|| bad("archive entry has no digest"))
    }
    fn labels(&self) -> Result<BTreeMap<String, String>> {
        let mut labels: BTreeMap<String, String> = [
            ("kind".into(), self.kind.clone()),
            ("table".into(), self.table.clone()),
            ("created".into(), self.created.clone()),
            (
                "owners".into(),
                serde_json::to_string(&self.owners).map_err(bad)?,
            ),
        ]
        .into();
        if let Some(hash) = &self.ciphertext {
            labels.insert("ciphertext_sha256".into(), hash.clone());
        }
        if let Some(proof) = &self.proof {
            labels.insert(
                "row_digest".into(),
                serde_json::to_string(proof).map_err(bad)?,
            );
        }
        if self.namespace.is_some() {
            labels.insert("filed_under".into(), self.tenant.clone());
        }
        if let Some(id) = &self.manifest {
            labels.insert("manifest_row".into(), id.clone());
        }
        Ok(labels)
    }
    fn statement(&self, state: &str) -> Result<Vec<u8>> {
        Ok(crate::attestation::manifest_statement(
            &self.id,
            self.namespace.as_deref().unwrap_or(&self.tenant),
            &self.blob()?.digest,
            "archive",
            state,
            &self.labels()?,
        ))
    }
    fn sign(&mut self, f: &Archive, state: &str) -> Result<()> {
        self.signature = f
            .notary
            .as_ref()
            .map(|n| n.sign(&self.statement(state)?))
            .transpose()?;
        Ok(())
    }
    fn validate(&self, f: &Archive, state: &str) -> Result<()> {
        match (&f.notary, &self.signature) {
            (Some(n), Some(s)) => n.verify(&self.statement(state)?, s),
            (None, None) => Ok(()),
            _ => Err(bad("archive signature or verification key missing")),
        }
    }
}
impl Transaction<'_> {
    fn typed_labels(&self, e: &Entry) -> Result<BTreeMap<String, String>> {
        let mut labels = e.labels()?;
        labels.insert("row".into(), e.id.clone());
        let name = if e.kind == "access" {
            "access".to_owned()
        } else {
            let domains = crate::domain::Domains::from_policies(self.policies)?;
            self.policies
                .get(&e.kind)
                .and_then(|p| p.entity["domain"].as_u64())
                .and_then(|n| u8::try_from(n).ok())
                .map(|domain| domains.name(domain))
                .unwrap_or_else(|| e.kind.clone())
        };
        labels.insert("Kind".into(), name);
        Ok(labels)
    }
    fn begin_blob(&mut self, files: &Archive, e: &mut Entry) -> Result<()> {
        let labels = self.typed_labels(e)?;
        let record = self.archive_intent(
            e.namespace.as_deref().unwrap_or(&e.tenant),
            "archive",
            labels,
            |_, r| {
                files
                    .notary
                    .as_ref()
                    .map(|n| n.sign(&r.statement()))
                    .transpose()
            },
        )?;
        e.manifest = paydirt::format_id(&record.id);
        Ok(())
    }
    fn settle_blob(
        &mut self,
        files: &Archive,
        e: &Entry,
        old: crate::manifest::State,
        new: crate::manifest::State,
    ) -> Result<()> {
        let key = e
            .manifest
            .as_deref()
            .and_then(paydirt::parse_id)
            .ok_or_else(|| bad("invalid manifest row id"))?;
        let labels = self.typed_labels(e)?;
        let digest = e.blob()?.digest;
        self.change_archive_record(
            key,
            old,
            |r| {
                r.labels = labels;
                r.digest = digest;
                r.state = new;
                Ok(())
            },
            |was, r| {
                if let Some(was) = was {
                    was.verify(files.notary.as_ref())?;
                }
                files
                    .notary
                    .as_ref()
                    .map(|n| n.sign(&r.statement()))
                    .transpose()
            },
        )?
        .ok_or_else(|| Fault::aborted("manifest row changed during transition"))?;
        Ok(())
    }
    fn check_archive_entry(&mut self, files: &Archive, e: &Entry, state: &str) -> Result<()> {
        e.validate(files, state)?;
        if let Some(key) = &e.manifest {
            let record = self.archive_record(
                paydirt::parse_id(key).ok_or_else(|| bad("invalid manifest row ID"))?,
            )?;
            record.verify(files.notary.as_ref())?;
            if record.state.name() != state
                || record.namespace != e.blob()?.namespace
                || record.digest != e.blob()?.digest
                || record.labels != self.typed_labels(e)?
            {
                return Err(bad("archived table differs from its recorded blob"));
            }
        }
        Ok(())
    }
    fn removal(&mut self, f: &Archive, e: &Entry) -> Result<()> {
        use crate::manifest::State::*;
        if e.manifest.is_some() {
            self.settle_blob(f, e, Present, Erasing)?;
            self.settle_blob(f, e, Erasing, Erased)?;
        }
        if f.notary.is_some() {
            let mut tombstone = e.clone();
            tombstone.sign(f, "erased")?;
            self.trail_set(
                "paydirt_trail_removed",
                &format!("{}/{}", e.tenant, e.digest.as_deref().unwrap()),
                Some(&tombstone),
            )?;
        }
        Ok(())
    }
}
#[derive(Serialize, Debug)]
pub struct Review {
    pub database_checkpoint: u64,
    pub archive_checkpoint: u64,
    pub of: String,
    pub unaccounted: Vec<crate::attestation::Blob>,
    pub added: Vec<crate::attestation::Blob>,
}
use crate::attestation::{Acceptance, Blob, Head, History};
type Inventory = (BTreeSet<Blob>, BTreeSet<Blob>);
impl Transaction<'_> {
    fn archive_inventory(&mut self, f: &Archive, advance: bool, behind: bool) -> Result<Inventory> {
        let mut current = BTreeSet::new();
        let mut removed = BTreeSet::new();
        for mut e in self.trail_docs::<Entry>("paydirt_trail_archive")? {
            if behind && let Some(key) = e.manifest.as_deref().and_then(paydirt::parse_id) {
                if let Err(fault) = self.archive_record(key) {
                    if fault.code != "not_found" {
                        return Err(fault);
                    }
                    e.validate(f, "present")?;
                    continue;
                }
            }
            self.check_archive_entry(f, &e, "present")?;
            f.read(e.digest.as_deref().ok_or_else(|| bad("missing digest"))?)?;
            if advance && e.manifest.is_none() {
                self.begin_blob(f, &mut e)?;
                self.settle_blob(
                    f,
                    &e,
                    crate::manifest::State::Adding,
                    crate::manifest::State::Present,
                )?;
                e.sign(f, "present")?;
                self.trail_set("paydirt_trail_archive", &e.id, Some(&e))?;
            }
            current.insert(e.blob()?);
        }
        for e in self.trail_docs::<Entry>("paydirt_trail_removed")? {
            self.check_archive_entry(f, &e, "erased")?;
            removed.insert(e.blob()?);
        }
        for row in self.archive_records()? {
            row.verify(f.notary.as_ref())?;
            let blob = crate::attestation::Blob {
                namespace: row.namespace,
                digest: row.digest,
            };
            match row.state {
                crate::manifest::State::Present => {
                    let digest = blob
                        .digest
                        .strip_prefix("sha256:")
                        .ok_or_else(|| bad("invalid manifest digest"))?;
                    f.read(digest)?;
                    current.insert(blob);
                }
                crate::manifest::State::Erased => {
                    removed.insert(blob);
                }
                _ => return Err(bad("manifest contains an unfinished transition")),
            }
        }
        Ok((current, removed))
    }
    fn compare_checkpoint(
        &mut self,
        files: &Archive,
        advance: bool,
    ) -> Result<(Review, History, Inventory)> {
        self.lock_trail()?;
        let signer = files
            .notary
            .as_ref()
            .ok_or_else(|| bad("checkpoint requires a configured signer and witness"))?;
        let history = signer.history(&files.root, advance)?;
        let external = history.head.as_ref().map_or(0, |h| h.k);
        let counter = self.checkpoint_counter()?;
        let legacy =
            counter.is_none() && self.trail_docs::<bool>("paydirt_trail_format")?.is_empty();
        let mut database = counter.unwrap_or(0);
        let inventory = self.archive_inventory(
            files,
            advance && database == external,
            database < external && !legacy,
        )?;
        if legacy && inventory.0 == history.latest {
            database = external;
        }
        let unaccounted = history
            .latest
            .difference(&inventory.0)
            .filter(|b| !inventory.1.contains(*b))
            .cloned()
            .collect::<Vec<_>>();
        let added = inventory
            .0
            .difference(&history.latest)
            .cloned()
            .collect::<Vec<_>>();
        let state = serde_json::to_vec(&(database, &history.head, &inventory.0, &inventory.1))
            .map_err(bad)?;
        let review = Review {
            database_checkpoint: database,
            archive_checkpoint: external,
            of: format!("sha256:{}", Archive::digest(&state)),
            unaccounted,
            added,
        };
        Ok((review, history, inventory))
    }
    fn adopt_checkpoint_blob(&mut self, f: &Archive, blob: &Blob) -> Result<()> {
        let digest = blob
            .digest
            .strip_prefix("sha256:")
            .ok_or_else(|| bad("invalid checkpoint digest"))?;
        let raw = f.read(digest)?;
        let row: Value = serde_json::from_slice(&raw).map_err(bad)?;
        let id =
            row_id(&row["id"]).ok_or_else(|| bad("checkpoint blob has no trail row identifier"))?;
        let source = if row.get("objectId").is_some() {
            "OWN_AUDIT"
        } else {
            "OWN_ACCESS"
        };
        let table = self
            .policies
            .iter()
            .find(|(_, p)| p.entity["own"] == source)
            .ok_or_else(|| bad("missing trail source"))?
            .0
            .clone();
        let kind = if source == "OWN_ACCESS" {
            "access".into()
        } else {
            self.policies
                .iter()
                .find(|(_, p)| p.entity["domain"] == row["domain"])
                .ok_or_else(|| bad("unknown kind in checkpoint object"))?
                .0
                .clone()
        };
        let tenant = row_id(&row["tenantId"])
            .unwrap_or_else(|| "00000000-0000-0000-0000-000000000000".into());
        let owners = ["tenantId", "actorTenantId", "counterpartTenantId"]
            .into_iter()
            .filter_map(|k| row_id(&row[k]))
            .collect();
        let mut e = Entry {
            id,
            manifest: None,
            kind,
            created: row["dateCreated"]
                .as_str()
                .ok_or_else(|| bad("missing checkpoint object time"))?
                .into(),
            namespace: (blob.namespace != tenant).then(|| blob.namespace.clone()),
            tenant,
            owners,
            table,
            digest: Some(digest.into()),
            proof: None,
            ciphertext: None,
            signature: None,
        };
        if source == "OWN_AUDIT" {
            e.proof = Some(evidence(&row));
            e.ciphertext = Some(f.ciphertext_digest(digest)?);
        }
        self.begin_blob(f, &mut e)?;
        self.settle_blob(
            f,
            &e,
            crate::manifest::State::Adding,
            crate::manifest::State::Present,
        )?;
        e.sign(f, "present")?;
        self.trail_set("paydirt_trail_archive", &e.id, Some(&e))
    }
    fn pending_checkpoint(&mut self) -> Result<Option<(u64, String)>> {
        Ok(self
            .trail_docs::<(u64, String)>("paydirt_checkpoint_pending")?
            .into_iter()
            .next())
    }
    fn membership_hash(members: &BTreeSet<Blob>) -> Result<String> {
        Ok(Archive::digest(&serde_json::to_vec(members).map_err(bad)?))
    }
    // Commit the intended set before publishing to a witness outside SQL.
    fn prepare_checkpoint(&mut self, f: &Archive) -> Result<()> {
        let (review, history, (current, _)) = self.compare_checkpoint(f, true)?;
        if review.database_checkpoint != review.archive_checkpoint {
            if self.pending_matches(&review, &history, &current)? {
                return Ok(());
            }
            return Err(bad(format!(
                "manifest counter is behind or ahead; review {}",
                review.of
            )));
        }
        if history.head.is_none() || history.latest != current {
            let pending = (review.database_checkpoint, Self::membership_hash(&current)?);
            self.trail_set("paydirt_checkpoint_pending", "deployment", Some(&pending))?;
        }
        Ok(())
    }
    fn pending_matches(
        &mut self,
        r: &Review,
        history: &History,
        current: &BTreeSet<Blob>,
    ) -> Result<bool> {
        Ok(
            r.database_checkpoint.checked_add(1) == Some(r.archive_checkpoint)
                && current == &history.latest
                && self.pending_checkpoint()?
                    == Some((r.database_checkpoint, Self::membership_hash(current)?)),
        )
    }
    fn checkpoint(
        &mut self,
        f: &Archive,
        advance: bool,
        accepted: Option<Acceptance>,
    ) -> Result<Option<Head>> {
        let (review, history, (mut current, removed)) = self.compare_checkpoint(f, advance)?;
        if let Some(a) = &accepted {
            if a.by.trim().is_empty()
                || a.why.trim().is_empty()
                || a.of != review.of
                || a.restored != review.database_checkpoint
                || a.restored >= review.archive_checkpoint
            {
                return Err(bad(
                    "acceptance requires this reviewed restore, its checkpoint number, operator and reason",
                ));
            }
            for blob in &review.unaccounted {
                self.adopt_checkpoint_blob(f, blob)?;
                current.insert(blob.clone());
            }
        } else if review.database_checkpoint != review.archive_checkpoint
            && !(advance && self.pending_matches(&review, &history, &current)?)
        {
            return Err(bad(format!(
                "manifest counter {} differs from the archive at {}: review {}",
                review.database_checkpoint, review.archive_checkpoint, review.of
            )));
        }
        let result =
            if advance && accepted.is_none() && history.head.is_some() && history.latest == current
            {
                history.head
            } else {
                f.notary
                    .as_ref()
                    .unwrap()
                    .finish_chain(&f.root, history, current, removed, advance, accepted)?
            };
        if advance && let Some(head) = &result {
            self.set_checkpoint_counter(head.k)?;
            self.trail_set("paydirt_trail_format", "typed", Some(&true))?;
            self.trail_clear("paydirt_checkpoint_pending", "deployment")?;
        }
        Ok(result)
    }
}
impl Store {
    pub fn attest_trail(&mut self, f: &Archive, advance: bool) -> Result<Option<Head>> {
        if advance {
            self.transaction(&Scope::Internal, |tx| tx.prepare_checkpoint(f))?;
        }
        self.transaction(&Scope::Internal, |tx| tx.checkpoint(f, advance, None))
    }
    pub fn review_trail(&mut self, f: &Archive) -> Result<Review> {
        self.transaction(&Scope::Internal, |tx| {
            tx.compare_checkpoint(f, false).map(|v| v.0)
        })
    }
    pub fn accept_trail_restore(&mut self, f: &Archive, acceptance: Acceptance) -> Result<Head> {
        self.transaction(&Scope::Internal, |tx| {
            tx.checkpoint(f, true, Some(acceptance))
        })?
        .ok_or_else(|| bad("restore did not produce a checkpoint"))
    }
}

impl Transaction<'_> {
    pub(crate) fn archived_digests(
        &mut self,
        files: &Archive,
        kind: &str,
        object: Option<[u8; 16]>,
    ) -> Result<Vec<Value>> {
        self.lock_trail()?;
        let mut rows = Vec::new();
        let encoded = object.map(|id| value::encoded(&id));
        for e in self.trail_docs::<Entry>("paydirt_trail_archive")? {
            self.check_archive_entry(files, &e, "present")?;
            if e.kind != kind {
                continue;
            }
            let row = if let (Some(_), Some(proof)) = (&files.notary, &e.proof) {
                if files.ciphertext_digest(
                    e.digest
                        .as_deref()
                        .ok_or_else(|| bad("missing archive digest"))?,
                )? != *e
                    .ciphertext
                    .as_ref()
                    .ok_or_else(|| bad("missing ciphertext digest"))?
                {
                    return Err(bad("signed ciphertext digest mismatch"));
                }
                proof.clone()
            } else {
                serde_json::from_slice(
                    &files.read(
                        e.digest
                            .as_deref()
                            .ok_or_else(|| bad("missing archive digest"))?,
                    )?,
                )
                .map_err(bad)?
            };
            if encoded.as_ref().is_none_or(|id| &row["objectId"] == id) {
                rows.push(row);
            }
        }
        Ok(rows)
    }
}
