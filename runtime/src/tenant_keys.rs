//! Per-tenant age recipients. Archive writers need only the master's public keys.
use crate::{Fault, Result, archive_files::Archive, attestation::write_once};
use age::{secrecy::ExposeSecret, x25519};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn bad(reason: impl ToString) -> Fault {
    Fault::precondition(reason.to_string())
}
fn encrypt(recipients: &[x25519::Recipient], data: &[u8]) -> Result<Vec<u8>> {
    let encoder =
        age::Encryptor::with_recipients(recipients.iter().map(|r| r as &dyn age::Recipient))
            .map_err(bad)?;
    let mut out = Vec::new();
    let mut writer = encoder.wrap_output(&mut out).map_err(bad)?;
    writer.write_all(data).map_err(bad)?;
    writer.finish().map_err(bad)?;
    Ok(out)
}
fn decrypt(ids: &[x25519::Identity], data: &[u8]) -> Result<Vec<u8>> {
    let decoder = age::Decryptor::new(data).map_err(bad)?;
    let mut reader = decoder
        .decrypt(ids.iter().map(|id| id as &dyn age::Identity))
        .map_err(|_| bad("no configured identity opens this archive key"))?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out).map_err(bad)?;
    Ok(out)
}
#[derive(Serialize, Deserialize)]
struct TenantKey {
    #[serde(default)]
    after: Option<DateTime<Utc>>,
    recipient: String,
    wrapped: String,
}
impl TenantKey {
    fn public(&self) -> Result<x25519::Recipient> {
        self.recipient.parse().map_err(bad)
    }
}
pub struct TenantKeys {
    root: PathBuf,
    masters: Vec<x25519::Recipient>,
    identities: Vec<x25519::Identity>,
}
impl TenantKeys {
    pub fn new(
        root: impl Into<PathBuf>,
        masters: Vec<x25519::Recipient>,
        identities: Vec<x25519::Identity>,
    ) -> Result<Self> {
        if masters.is_empty() {
            return Err(Fault::invalid(
                "at least one archive master recipient is required",
            ));
        }
        Ok(Self {
            root: root.into(),
            masters,
            identities,
        })
    }
    fn spaces(owners: &BTreeSet<String>) -> Vec<&str> {
        if owners.is_empty() {
            vec!["deployment"]
        } else {
            owners.iter().map(String::as_str).collect()
        }
    }
    fn directory(&self, name: &str) -> Result<PathBuf> {
        if name != "deployment"
            && !paydirt::parse_id(name)
                .is_some_and(|id| paydirt::format_id(&id).as_deref() == Some(name))
        {
            return Err(bad("invalid tenant key namespace"));
        }
        Ok(self.root.join(name))
    }
    fn key(&self, dir: &Path) -> Result<Option<TenantKey>> {
        let path = dir.join("recipient.json");
        match fs::read(path) {
            Ok(data) => serde_json::from_slice(&data).map(Some).map_err(bad),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(bad(e)),
        }
    }
    fn left(&self, dir: &Path) -> Result<Option<DateTime<Utc>>> {
        let path = dir.join("departures");
        let entries = match fs::read_dir(path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(bad(e)),
        };
        let mut last = None;
        for file in entries {
            let path = file.map_err(bad)?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let at: DateTime<Utc> =
                serde_json::from_slice(&fs::read(path).map_err(bad)?).map_err(bad)?;
            last = Some(last.map_or(at, |prior: DateTime<Utc>| prior.max(at)));
        }
        Ok(last)
    }
    fn recipient(&self, name: &str, created: DateTime<Utc>) -> Result<Option<x25519::Recipient>> {
        let dir = self.directory(name)?;
        let after = self.left(&dir)?;
        if after.is_some_and(|left| created <= left) {
            return Ok(None);
        }
        if let Some(key) = self.key(&dir)? {
            return key.public().map(Some);
        }
        let id = x25519::Identity::generate();
        let secret = id.to_string();
        let key = TenantKey {
            after,
            recipient: id.to_public().to_string(),
            wrapped: STANDARD.encode(encrypt(&self.masters, secret.expose_secret().as_bytes())?),
        };
        let created = write_once(
            &dir.join("recipient.json"),
            &serde_json::to_vec(&key).map_err(bad)?,
        );
        // Concurrent first uses converge on the key whose atomic link won.
        match self.key(&dir)? {
            Some(persisted) => persisted.public().map(Some),
            None => {
                created?;
                Err(bad("tenant key disappeared during creation"))
            }
        }
    }
    pub(crate) fn seal(
        &self,
        mut doc: Value,
        owners: &BTreeSet<String>,
        created: DateTime<Utc>,
    ) -> Result<Value> {
        let row = doc
            .as_object_mut()
            .ok_or_else(|| bad("invalid archive document"))?;
        if row.contains_key("sealed") {
            return Ok(doc);
        }
        let contents: Map<_, _> = ["patch", "value"]
            .into_iter()
            .filter_map(|k| row.remove(k).map(|v| (k.to_string(), v)))
            .collect();
        if contents
            .values()
            .all(|v| v.is_null() || v.as_str() == Some(""))
        {
            row.extend(contents);
            return Ok(doc);
        }
        let mut recipients = Vec::new();
        for name in Self::spaces(owners) {
            if let Some(r) = self.recipient(name, created)? {
                recipients.push(r);
            }
        }
        if recipients.is_empty() {
            return Err(bad("every owner of this historical row has departed"));
        }
        row.insert(
            "sealed".into(),
            STANDARD
                .encode(encrypt(
                    &recipients,
                    &serde_json::to_vec(&contents).map_err(bad)?,
                )?)
                .into(),
        );
        Ok(doc)
    }
    pub fn open_row(&self, mut doc: Value, owners: &BTreeSet<String>) -> Result<Value> {
        let Some(sealed) = doc.get("sealed") else {
            return Ok(doc);
        };
        if self.identities.is_empty() {
            return Err(bad(
                "archived contents are sealed; a master identity is required",
            ));
        }
        let cipher = STANDARD
            .decode(
                sealed
                    .as_str()
                    .ok_or_else(|| bad("invalid sealed content"))?,
            )
            .map_err(bad)?;
        let mut ids = Vec::new();
        for name in Self::spaces(owners) {
            if let Some(key) = self.key(&self.directory(name)?)? {
                let encoded = STANDARD.decode(&key.wrapped).map_err(bad)?;
                let decrypted = age::secrecy::SecretString::from(
                    String::from_utf8(decrypt(&self.identities, &encoded)?)
                        .map_err(|_| bad("invalid tenant identity"))?,
                );
                let id: x25519::Identity = decrypted
                    .expose_secret()
                    .parse()
                    .map_err(|_| bad("invalid tenant identity"))?;
                if key.recipient != id.to_public().to_string() {
                    return Err(bad("tenant identity differs from the recorded recipient"));
                }
                ids.push(id);
            }
        }
        if ids.is_empty() {
            return Err(bad("tenant archive key is gone"));
        }
        let values: Map<String, Value> =
            serde_json::from_slice(&decrypt(&ids, &cipher)?).map_err(bad)?;
        let row = doc
            .as_object_mut()
            .ok_or_else(|| bad("invalid archive record"))?;
        row.remove("sealed");
        for (name, value) in values {
            if !matches!(name.as_str(), "value" | "patch") || row.contains_key(&name) {
                return Err(bad("unexpected sealed archive member"));
            }
            row.insert(name, value);
        }
        Ok(doc)
    }
    pub(crate) fn has_key(&self, tenant: &str) -> Result<bool> {
        Ok(self.key(&self.directory(tenant)?)?.is_some())
    }
    pub(crate) fn destroy(&self, tenant: &str, at: DateTime<Utc>) -> Result<()> {
        let dir = self.directory(tenant)?;
        let content = serde_json::to_vec(&at).map_err(bad)?;
        write_once(
            &dir.join("departures")
                .join(format!("{}.json", Archive::digest(&content))),
            &content,
        )?;
        if self
            .key(&dir)?
            .and_then(|key| key.after)
            .is_some_and(|generation| generation >= at)
        {
            return Ok(());
        }
        match fs::remove_file(dir.join("recipient.json")) {
            Ok(()) => File::open(&dir).and_then(|f| f.sync_all()).map_err(bad),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(bad(e)),
        }
    }
}
impl Archive {
    pub fn with_tenant_keys(mut self, keys: TenantKeys) -> Result<Self> {
        fn resolve(path: &Path) -> Result<PathBuf> {
            if path.exists() {
                return path.canonicalize().map_err(bad);
            }
            let absolute = std::path::absolute(path).map_err(bad)?;
            let parent = absolute
                .parent()
                .ok_or_else(|| bad("invalid archive path"))?;
            let name = absolute
                .file_name()
                .ok_or_else(|| bad("invalid archive path"))?;
            Ok(resolve(parent)?.join(name))
        }
        let archive = resolve(&self.root)?;
        let directory = resolve(&keys.root)?;
        if directory.starts_with(&archive) || archive.starts_with(&directory) {
            return Err(Fault::invalid(
                "archive and tenant key store need separate directories",
            ));
        }
        self.tenant_keys = Some(keys);
        Ok(self)
    }
    pub(crate) fn environment_keys(self) -> Result<Self> {
        use paydirt::environment::*;
        let Some(root) = std::env::var_os(PAYDIRT_TENANT_KEY_DIR) else {
            return Ok(self);
        };
        let names: Vec<String> =
            serde_json::from_str(&std::env::var(PAYDIRT_ARCHIVE_RECIPIENTS).map_err(bad)?)
                .map_err(bad)?;
        let masters = names
            .iter()
            .map(|v| v.parse().map_err(bad))
            .collect::<Result<Vec<_>>>()?;
        let mut identities = Vec::new();
        if let Some(path) = std::env::var_os(PAYDIRT_ARCHIVE_IDENTITIES) {
            let text = age::secrecy::SecretString::from(fs::read_to_string(path).map_err(bad)?);
            for line in text
                .expose_secret()
                .lines()
                .map(str::trim)
                .filter(|v| !v.is_empty() && !v.starts_with('#'))
            {
                identities.push(
                    line.parse()
                        .map_err(|_| bad("invalid archive master identity file"))?,
                );
            }
        }
        self.with_tenant_keys(TenantKeys::new(PathBuf::from(root), masters, identities)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    #[ignore = "requires a pinned Go checkout using PAYDAY_CHECKOUT"]
    fn upstream_age_opens_rust_rows_and_rust_opens_go_rows() {
        use std::process::{Command, Stdio};
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("scripts/fixtures/age-interop.go");
        let dir = tempfile::tempdir().unwrap();
        let master = x25519::Identity::generate();
        let keys =
            TenantKeys::new(dir.path(), vec![master.to_public()], vec![master.clone()]).unwrap();
        let tenant = paydirt::format_id(&paydirt::mint(1)).unwrap();
        let owners = BTreeSet::from([tenant.clone()]);
        let row = keys
            .seal(json!({"value":"ZnJvbSBSdXN0"}), &owners, Utc::now())
            .unwrap();
        let key = keys
            .key(&keys.directory(&tenant).unwrap())
            .unwrap()
            .unwrap();
        let secret = master.to_string();
        let data =
            json!({"master":secret.expose_secret(),"wrapped":key.wrapped,"sealed":row["sealed"]});
        let mut go = Command::new("go")
            .arg("run")
            .arg(fixture)
            .current_dir(std::env::var_os("PAYDAY_CHECKOUT").unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        serde_json::to_writer(go.stdin.take().unwrap(), &data).unwrap();
        let out = go.wait_with_output().unwrap();
        assert!(out.status.success());
        let sealed: String = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            keys.open_row(json!({"sealed":sealed}), &owners).unwrap(),
            json!({"value":"ZnJvbSBHbw=="})
        );
    }
}
