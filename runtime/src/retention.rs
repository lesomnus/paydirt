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
    pub carried: Vec<String>,
    pub removed: Vec<String>,
    pub held: Vec<String>,
    pub blanked: Vec<String>,
    pub owed: Vec<String>,
    pub exported: Vec<String>,
    pub bundle: Option<String>,
    pub retired: bool,
}
impl Policy {
    fn resolve(&self, policies: &crate::Policies) -> Result<Self> {
        let domains = crate::domain::Domains::from_policies(policies)?;
        let canonical = |name: &str| -> Result<String> {
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
        };
        let mut by = BTreeMap::new();
        for (name, value) in &self.by {
            if by.insert(canonical(name)?, value.clone()).is_some() {
                return Err(Fault::invalid("trail policy names the same kind twice"));
            }
        }
        let mut held = BTreeSet::new();
        for name in &self.held {
            let kind = if name == "*" {
                name.clone()
            } else {
                canonical(name)?
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
CREATE TABLE IF NOT EXISTS paydirt_trail_gc (id TEXT PRIMARY KEY, doc TEXT NOT NULL);";
impl Transaction<'_> {
    fn lock_trail(&mut self) -> Result<()> {
        if self.db.dialect() == Dialect::Postgres {
            self.db.rows(Query::new(
                "SELECT '1' AS value FROM pg_advisory_xact_lock(102774192,51)",
            ))?;
        }
        self.db.batch(TABLES)
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
        self.trail_set(
            "paydirt_trail_owed",
            &format!("{tenant}/{subject}"),
            Some(&(&tenant, &subject)),
        )
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
        let mut rows = Vec::new();
        for (name, p) in self
            .policies
            .iter()
            .filter(|(_, p)| matches!(p.entity["own"].as_str(), Some("OWN_AUDIT" | "OWN_ACCESS")))
        {
            let access = p.entity["own"] == "OWN_ACCESS";
            let entity = self.model.entity(name).map_err(bad)?;
            let mut q = Query::new("");
            let owner = q.param(
                Bind::Bytes(paydirt::parse_id(tenant).unwrap().to_vec()),
                self.db.dialect(),
            );
            let columns = if shared {
                &["tenant_id", "actor_tenant_id", "counterpart_tenant_id"][..]
            } else {
                &["tenant_id"][..]
            };
            let owners = columns
                .iter()
                .filter_map(|n| entity.field(n).ok())
                .map(|f| format!("r.{}={owner}", quote(&f.column())))
                .collect::<Vec<_>>()
                .join(" OR ");
            q.sql = format!(
                "SELECT {} AS value FROM {} r WHERE ({owners}) ORDER BY r.{}",
                self.project_sql(entity)?,
                quote(&entity.table()),
                quote(&entity.key().map_err(bad)?.column())
            );
            if self.db.dialect() == Dialect::Postgres {
                q.sql.push_str(" FOR UPDATE")
            }
            for raw in self.db.rows(q)? {
                let row = self.decode_row(entity, &raw)?;
                let id = row_id(&row["id"]).ok_or_else(|| bad("invalid trail row ID"))?;
                let kind = if access {
                    "access".into()
                } else {
                    self.policies
                        .iter()
                        .find(|(_, p)| p.entity["domain"] == row["domain"])
                        .map(|(n, _)| n.clone())
                        .unwrap_or_else(|| format!("domain.{}", row["domain"]))
                };
                let owners = ["tenantId", "actorTenantId", "counterpartTenantId"]
                    .into_iter()
                    .filter_map(|k| row_id(&row[k]))
                    .collect();
                rows.push((
                    Entry {
                        id,
                        tenant: row_id(&row["tenantId"])
                            .unwrap_or_else(|| "00000000-0000-0000-0000-000000000000".into()),
                        kind,
                        created: row["dateCreated"]
                            .as_str()
                            .ok_or_else(|| bad("missing trail time"))?
                            .into(),
                        owners,
                        namespace: None,
                        proof: None,
                        ciphertext: None,
                        table: name.clone(),
                        digest: None,
                        signature: None,
                    },
                    row,
                ));
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
    fn collect_gc(&mut self, files: &Archive) -> Result<()> {
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
        let tenant = uuid(&request.tenant)?;
        if !paydirt::has_domain(&paydirt::parse_id(&tenant).unwrap(), 1) {
            return Err(Fault::invalid("not a tenant ID"));
        }
        let dry = request.dry_run;
        let out = self.transaction(&Scope::Internal, |tx| {
            run_pass(tx, files, recipient, &request, tenant, dry)
        })?;

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
) -> Result<Report> {
    tx.lock_trail()?;
    let now = (tx.inputs.clock)();
    let mut states: BTreeMap<String, State> = tx
        .trail_docs::<(String, State)>("paydirt_trail_policy")?
        .into_iter()
        .collect();
    let mut state = states.get(&tenant).cloned().unwrap_or_default();
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
    let mut owed: BTreeSet<String> = tx
        .trail_docs::<(String, String)>("paydirt_trail_owed")?
        .into_iter()
        .filter(|(t, _)| t == &tenant)
        .map(|(_, o)| o)
        .collect();
    if let Action::Forget { subject } = &request.action {
        let subject = uuid(subject)?;
        owed.insert(subject.clone());
        if !dry {
            tx.trail_set(
                "paydirt_trail_owed",
                &format!("{tenant}/{subject}"),
                Some(&(&tenant, &subject)),
            )?;
        }
    }
    let mut waiting = BTreeSet::new();
    let mut exported = Vec::new();
    let mut carry = crate::integrity::CarryHeads::default();
    let shared = !matches!(request.action, Action::Retire | Action::Purge);
    let mut rows = tx.live_trail(&tenant, shared)?;
    for entry in tx.trail_docs::<Entry>("paydirt_trail_archive")?.into_iter() {
        entry.validate(files, "present")?;
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
        let held = e.owners.iter().any(|t| {
            states
                .get(t)
                .is_some_and(|s| s.policy.held.contains("*") || s.policy.held.contains(&e.kind))
        });
        let default_policy = Policy::default();
        let mut collect = true;
        let mut destroy = true;
        for owner in &e.owners {
            let keep = states
                .get(owner)
                .map(|s| &s.policy)
                .unwrap_or(&default_policy)
                .keep(&e.kind);
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
        let target = matches!(request.action, Action::Purge)
            || matches!(request.action, Action::Collect | Action::Retire)
                && (destroy || state.retired && k.retain_ms.is_none() && k.destroy_ms.is_none());
        if matches!(request.action, Action::Export { .. }) {
            out.exported.push(e.id.clone());
            exported.push(row);
            continue;
        }
        let shared_departure = e.owners.len() > 1
            && (matches!(request.action, Action::Purge | Action::Retire)
                || state.retired && e.tenant == tenant);
        if held && (target || shared_departure || forget && !subjects.is_empty()) {
            out.held.push(e.id.clone());
            waiting.extend(subjects.iter().cloned());
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
            && ["patch", "value"]
                .iter()
                .any(|k| row[k].as_str().is_some_and(|s| !s.is_empty()));
        if blank {
            out.blanked.push(e.id.clone());
            for key in ["patch", "value"] {
                if row.get(key).is_some() {
                    row[key] = json!("")
                }
            }
        }
        let transfer = !archived
            && matches!(
                request.action,
                Action::Collect | Action::Retire | Action::Purge
            )
            && (collect || state.retired || target && held);
        if transfer {
            out.archived.push(e.id.clone());
        }
        if !dry && (transfer || archived && blank) {
            let hash = files.stage(&serde_json::to_vec(&row).map_err(bad)?)?;
            if e.digest.is_some() {
                tx.removal(files, &e)?;
            }
            if !archived && e.owners.len() > 1 {
                e.namespace = Some("shared".into());
            }
            if let Some(old) = e.digest.replace(hash) {
                tx.trail_set("paydirt_trail_gc", &old, Some(&old))?;
            }
            if files.notary.is_some() && row.get("objectId").is_some() {
                let proof: serde_json::Map<_, _> = [
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
                .filter_map(|name| row.get(name).map(|value| (name.to_string(), value.clone())))
                .collect();
                e.proof = Some(Value::Object(proof));
                e.ciphertext =
                    Some(files.ciphertext_digest(e.digest.as_deref().expect("staged digest"))?);
            }
            e.sign(files, "present")?;
            tx.trail_set("paydirt_trail_archive", &e.id, Some(&e))?;
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
        for o in owed.difference(&waiting) {
            if !dry {
                tx.forget_seal(&paydirt::parse_id(o).ok_or_else(|| bad("invalid subject"))?)?;
                tx.trail_clear("paydirt_trail_owed", &format!("{tenant}/{o}"))?;
            }
        }
        out.owed = waiting.into_iter().collect();
    } else {
        out.owed = owed.into_iter().collect();
    }
    Ok(out)
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
    fn statement(&self, state: &str) -> Result<Vec<u8>> {
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
        Ok(crate::attestation::manifest_statement(
            &self.id,
            self.namespace.as_deref().unwrap_or(&self.tenant),
            &self.blob()?.digest,
            "archive",
            state,
            &labels,
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
    fn removal(&mut self, f: &Archive, e: &Entry) -> Result<()> {
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
impl Store {
    pub fn attest_trail(
        &mut self,
        f: &Archive,
        advance: bool,
    ) -> Result<Option<crate::attestation::Head>> {
        let n = f
            .notary
            .as_ref()
            .ok_or_else(|| bad("checkpoint requires a configured signer and witness"))?;
        self.transaction(&Scope::Internal, |tx| {
            tx.lock_trail()?;
            let mut current = BTreeSet::new();
            let mut removed = BTreeSet::new();
            for e in tx.trail_docs::<Entry>("paydirt_trail_archive")? {
                e.validate(f, "present")?;
                f.read(e.digest.as_deref().ok_or_else(|| bad("missing digest"))?)?;
                current.insert(e.blob()?);
            }
            for e in tx.trail_docs::<Entry>("paydirt_trail_removed")? {
                e.validate(f, "erased")?;
                removed.insert(e.blob()?);
            }
            n.check_chain(&f.root, current, removed, advance)
        })
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
            e.validate(files, "present")?;
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
