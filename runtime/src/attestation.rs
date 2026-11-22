//! Signed archive statements and gzip checkpoints compatible with Go Payday.
use crate::{Fault, Result, archive_files::Archive};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use ed25519_dalek::{SigningKey, pkcs8::DecodePrivateKey};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use ring::signature;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
const ED: &[u8] = &[
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];
fn bad(e: impl ToString) -> Fault {
    Fault::precondition(e.to_string())
}
pub trait Signer: Send + Sync {
    fn public_der(&self) -> Vec<u8>;
    fn sign(&self, statement: &[u8]) -> Result<Vec<u8>>;
    fn sign_prehash(&self, _statement: &[u8]) -> Result<Vec<u8>> {
        Err(bad("the signing provider does not support Ed25519ph"))
    }
}
pub struct Ed25519(SigningKey);
impl Ed25519 {
    pub fn from_pkcs8(bytes: &[u8]) -> Result<Self> {
        SigningKey::from_pkcs8_der(bytes).map(Self).map_err(bad)
    }
    pub fn from_seed(seed: &[u8]) -> Result<Self> {
        SigningKey::try_from(seed).map(Self).map_err(bad)
    }
}
impl Signer for Ed25519 {
    fn public_der(&self) -> Vec<u8> {
        [ED, self.0.verifying_key().as_bytes()].concat()
    }
    fn sign(&self, statement: &[u8]) -> Result<Vec<u8>> {
        use ed25519_dalek::Signer;
        Ok(self.0.sign(statement).to_vec())
    }
    fn sign_prehash(&self, statement: &[u8]) -> Result<Vec<u8>> {
        self.0
            .sign_prehashed(sha2::Sha512::new_with_prefix(statement), None)
            .map(|s| s.to_vec())
            .map_err(bad)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    pub key: String,
    pub sig: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Blob {
    #[serde(rename = "ns")]
    pub namespace: String,
    pub digest: String,
}
impl Blob {
    pub fn of(namespace: &str, digest: &str) -> Self {
        Self {
            namespace: namespace.into(),
            digest: format!("sha256:{digest}"),
        }
    }
}
pub fn manifest_statement(
    key: &str,
    namespace: &str,
    digest: &str,
    intent: &str,
    state: &str,
    labels: &BTreeMap<String, String>,
) -> Vec<u8> {
    let mut s = b"payday.trail.Archived\n".to_vec();
    for field in [key, namespace, digest, intent, state]
        .into_iter()
        .chain(labels.iter().flat_map(|(k, v)| [k.as_str(), v.as_str()]))
    {
        s.extend(format!("{}:", field.len()).bytes());
        s.extend(field.as_bytes());
        s.push(b'\n');
    }
    s
}
fn hash(raw: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(raw)))
}
fn membership(v: &[Blob]) -> String {
    let mut b = Vec::new();
    for v in v {
        b.extend(v.namespace.bytes());
        b.push(b' ');
        b.extend(v.digest.bytes());
        b.push(b'\n');
    }
    hash(&b)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acceptance {
    pub of: String,
    pub by: String,
    pub why: String,
    pub restored: u64,
}
pub(crate) struct History {
    pub head: Option<Head>,
    pub latest: BTreeSet<Blob>,
}
#[derive(Serialize, Deserialize)]
pub struct Checkpoint {
    pub k: u64,
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prev: String,
    pub count: usize,
    pub digest: String,
    pub blobs: Vec<Blob>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<Acceptance>,
}
#[derive(Deserialize, Serialize)]
struct Envelope {
    checkpoint: Box<RawValue>,
    #[serde(flatten)]
    signature: Signature,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct Head {
    pub k: u64,
    pub digest: String,
}
pub struct Claim {
    pub head: Head,
    pub statement: String,
    pub signature: Signature,
}

pub trait Witness: Send + Sync {
    fn heads(&self) -> Result<BTreeMap<u64, Head>> {
        Ok(BTreeMap::new())
    }
    fn publish(&self, _head: &Head) -> Result<()> {
        Err(bad("witness needs a signed statement"))
    }
    fn submit(&self, _root: &Path, claim: &Claim, _signer: &dyn Signer) -> Result<()> {
        self.publish(&claim.head)
    }
    fn check(
        &self,
        root: &Path,
        history: &BTreeMap<u64, Claim>,
        signer: &dyn Signer,
        _trusted: &BTreeSet<Vec<u8>>,
        advance: bool,
    ) -> Result<()> {
        let witness = self.heads()?;
        for (k, heard) in &witness {
            if history.get(k).map(|c| &c.head) != Some(heard) {
                return Err(bad("external witness disagrees"));
            }
        }
        for (k, claim) in history {
            if !witness.contains_key(k) {
                if !advance {
                    return Err(bad("checkpoint has no witness receipt"));
                }
                self.submit(root, claim, signer)?;
            }
        }
        Ok(())
    }
}

pub struct Witnesses(pub Vec<Box<dyn Witness>>);
impl Witness for Witnesses {
    fn submit(&self, root: &Path, claim: &Claim, signer: &dyn Signer) -> Result<()> {
        for witness in &self.0 {
            witness.submit(root, claim, signer)?;
        }
        Ok(())
    }
    fn check(
        &self,
        root: &Path,
        history: &BTreeMap<u64, Claim>,
        signer: &dyn Signer,
        trusted: &BTreeSet<Vec<u8>>,
        advance: bool,
    ) -> Result<()> {
        for witness in &self.0 {
            witness.check(root, history, signer, trusted, advance)?;
        }
        Ok(())
    }
}
pub struct LogWitness;
impl Witness for LogWitness {
    fn submit(&self, root: &Path, claim: &Claim, _signer: &dyn Signer) -> Result<()> {
        eprintln!("paydirt checkpoint {} {}", claim.head.k, claim.head.digest);
        write_once(
            &root
                .join(".receipts/log")
                .join(format!("{:020}.json", claim.head.k)),
            &serde_json::to_vec(&claim.head).map_err(bad)?,
        )
    }
    fn check(
        &self,
        root: &Path,
        history: &BTreeMap<u64, Claim>,
        signer: &dyn Signer,
        _trusted: &BTreeSet<Vec<u8>>,
        advance: bool,
    ) -> Result<()> {
        DirectoryWitness(root.join(".receipts/log"))
            .heads()?
            .into_iter()
            .try_for_each(|(k, h)| {
                if history.get(&k).map(|c| &c.head) == Some(&h) {
                    Ok(())
                } else {
                    Err(bad("log receipt disagrees"))
                }
            })?;
        for (k, claim) in history {
            if !root
                .join(".receipts/log")
                .join(format!("{k:020}.json"))
                .exists()
            {
                if !advance {
                    return Err(bad("checkpoint has no log receipt"));
                }
                self.submit(root, claim, signer)?;
            }
        }
        Ok(())
    }
}
pub struct DirectoryWitness(pub PathBuf);
pub(crate) fn write_once(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| bad("invalid checkpoint directory"))?;
    fs::create_dir_all(dir).map_err(bad)?;
    if path.exists() {
        return if fs::read(path).map_err(bad)? == data {
            Ok(())
        } else {
            Err(bad("conflicting checkpoint"))
        };
    }
    let tmp = dir.join(format!(".tmp-{}", hex::encode(paydirt::mint(0))));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(bad)?;
    let result = (|| {
        f.write_all(data).and_then(|_| f.sync_all()).map_err(bad)?;
        match fs::hard_link(&tmp, path) {
            Ok(()) => {}
            Err(e)
                if e.kind() == std::io::ErrorKind::AlreadyExists
                    && fs::read(path).map_err(bad)? == data => {}
            Err(e) => return Err(bad(e)),
        };
        File::open(dir).and_then(|d| d.sync_all()).map_err(bad)
    })();
    drop(f);
    let _ = fs::remove_file(tmp);
    result
}
pub(crate) fn files(path: &Path, ext: &str) -> Result<BTreeMap<u64, PathBuf>> {
    let mut out = BTreeMap::new();
    let list = match fs::read_dir(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(bad(e)),
    };
    for entry in list {
        let path = entry.map_err(bad)?.path();
        if path.extension().and_then(|s| s.to_str()) == Some(ext) {
            let k = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| bad("invalid chain filename"))?;
            if out.insert(k, path).is_some() {
                return Err(bad("duplicate checkpoint"));
            }
        }
    }
    Ok(out)
}
impl Witness for DirectoryWitness {
    fn publish(&self, h: &Head) -> Result<()> {
        write_once(
            &self.0.join(format!("{:020}.json", h.k)),
            &serde_json::to_vec(h).map_err(bad)?,
        )
    }
    fn heads(&self) -> Result<BTreeMap<u64, Head>> {
        files(&self.0, "json")?
            .into_iter()
            .map(|(k, path)| {
                let h: Head = serde_json::from_slice(&fs::read(path).map_err(bad)?).map_err(bad)?;
                if k != h.k {
                    return Err(bad("incorrect witness index"));
                }
                Ok((k, h))
            })
            .collect()
    }
}
pub struct Notary {
    signer: Box<dyn Signer>,
    trusted: BTreeSet<Vec<u8>>,
    witness: Box<dyn Witness>,
}
impl Notary {
    pub fn new(
        signer: Box<dyn Signer>,
        mut trusted: BTreeSet<Vec<u8>>,
        witness: Box<dyn Witness>,
    ) -> Self {
        trusted.insert(signer.public_der());
        Self {
            signer,
            trusted,
            witness,
        }
    }
    pub fn sign(&self, doc: &[u8]) -> Result<Signature> {
        let s = Signature {
            key: STANDARD.encode(self.signer.public_der()),
            sig: STANDARD.encode(self.signer.sign(doc)?),
        };
        self.verify(doc, &s)?;
        Ok(s)
    }
    pub fn verify(&self, doc: &[u8], s: &Signature) -> Result<()> {
        let key = STANDARD.decode(&s.key).map_err(bad)?;
        let sig = STANDARD.decode(&s.sig).map_err(bad)?;
        if !self.trusted.contains(&key) {
            return Err(bad("untrusted signing key"));
        }
        let spki = spki::SubjectPublicKeyInfoRef::try_from(key.as_slice()).map_err(bad)?;
        let pk = spki
            .subject_public_key
            .as_bytes()
            .ok_or_else(|| bad("invalid signing key bits"))?;
        let algorithms: &[&dyn signature::VerificationAlgorithm] =
            match spki.algorithm.oid.to_string().as_str() {
                "1.3.101.112" => &[&signature::ED25519],
                "1.2.840.10045.2.1" => &[
                    &signature::ECDSA_P256_SHA256_ASN1,
                    &signature::ECDSA_P384_SHA256_ASN1,
                    &signature::ECDSA_P384_SHA384_ASN1,
                ],
                "1.2.840.113549.1.1.1" => &[
                    &signature::RSA_PKCS1_2048_8192_SHA256,
                    &signature::RSA_PSS_2048_8192_SHA256,
                ],
                _ => return Err(bad("unsupported checkpoint key")),
            };
        if !algorithms.iter().any(|alg| {
            signature::UnparsedPublicKey::new(*alg, pk)
                .verify(doc, &sig)
                .is_ok()
        }) {
            return Err(bad("signature mismatch"));
        }
        Ok(())
    }
    pub fn verify_point(&self, raw: &[u8], previous: Option<&Head>) -> Result<Checkpoint> {
        self.read_point(raw, previous).map(|(c, _)| c)
    }
    fn read_point(&self, raw: &[u8], previous: Option<&Head>) -> Result<(Checkpoint, Envelope)> {
        let mut body = Vec::new();
        GzDecoder::new(raw)
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut body)
            .map_err(bad)?;
        if body.len() > 16 * 1024 * 1024 {
            return Err(bad("checkpoint exceeds 16 MiB"));
        }
        let signed: Envelope = serde_json::from_slice(&body).map_err(bad)?;
        self.verify(signed.checkpoint.get().as_bytes(), &signed.signature)?;
        let c: Checkpoint = serde_json::from_str(signed.checkpoint.get()).map_err(bad)?;
        if c.count != c.blobs.len()
            || c.blobs.windows(2).any(|p| p[0] >= p[1])
            || c.digest != membership(&c.blobs)
            || c.k != previous.map_or(1, |h| h.k + 1)
            || c.prev != previous.map_or("", |h| &h.digest)
        {
            return Err(bad("invalid checkpoint membership or predecessor"));
        }
        Ok((c, signed))
    }
    pub fn check_chain(
        &self,
        root: &Path,
        blobs: BTreeSet<Blob>,
        removed: BTreeSet<Blob>,
        advance: bool,
    ) -> Result<Option<Head>> {
        let history = self.history(root, advance)?;
        self.finish_chain(root, history, blobs, removed, advance, None)
    }
    pub(crate) fn history(&self, root: &Path, advance: bool) -> Result<History> {
        let directory = root.join(".checkpoints");
        let mut history = BTreeMap::new();
        let mut head: Option<Head> = None;
        let mut latest = BTreeSet::new();
        for (k, path) in files(&directory, "gz")? {
            if fs::metadata(&path).map_err(bad)?.len() > 16 * 1024 * 1024 {
                return Err(bad("oversized checkpoint"));
            }
            let raw = fs::read(path).map_err(bad)?;
            let (c, signed) = self.read_point(&raw, head.as_ref())?;
            if c.k != k {
                return Err(bad("incorrect checkpoint filename"));
            }
            latest = c.blobs.into_iter().collect();
            let next = Head {
                k,
                digest: hash(&raw),
            };
            history.insert(
                k,
                Claim {
                    head: next.clone(),
                    statement: signed.checkpoint.get().into(),
                    signature: signed.signature,
                },
            );
            head = Some(next);
        }
        self.witness
            .check(root, &history, self.signer.as_ref(), &self.trusted, advance)?;
        Ok(History { head, latest })
    }
    pub(crate) fn finish_chain(
        &self,
        root: &Path,
        history: History,
        blobs: BTreeSet<Blob>,
        removed: BTreeSet<Blob>,
        advance: bool,
        accepted: Option<Acceptance>,
    ) -> Result<Option<Head>> {
        let directory = root.join(".checkpoints");
        let History { head, latest } = history;
        if !advance {
            if latest != blobs {
                return Err(bad("checkpoint differs from current archive"));
            }
            return Ok(head);
        }
        if latest.difference(&blobs).any(|b| !removed.contains(b)) {
            return Err(bad("missing previously checkpointed archive object"));
        }
        let blobs = blobs.into_iter().collect::<Vec<_>>();
        let k = head.as_ref().map_or(1, |h| h.k + 1);
        let c = Checkpoint {
            k,
            at: Utc::now(),
            prev: head.as_ref().map(|h| h.digest.clone()).unwrap_or_default(),
            count: blobs.len(),
            digest: membership(&blobs),
            blobs,
            accepted,
        };
        let doc = serde_json::to_string(&c).map_err(bad)?;
        let signature = self.sign(doc.as_bytes())?;
        let env = Envelope {
            checkpoint: RawValue::from_string(doc).map_err(bad)?,
            signature,
        };
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        serde_json::to_writer(&mut gz, &env).map_err(bad)?;
        let bytes = gz.finish().map_err(bad)?;
        write_once(&directory.join(format!("{k:020}.gz")), &bytes)?;
        let h = Head {
            k,
            digest: hash(&bytes),
        };
        let claim = Claim {
            head: h.clone(),
            statement: env.checkpoint.get().into(),
            signature: env.signature,
        };
        self.witness.submit(root, &claim, self.signer.as_ref())?;
        Ok(Some(h))
    }
}
impl Archive {
    pub fn with_notary(mut self, notary: Notary) -> Self {
        self.notary = Some(notary);
        self
    }
    pub fn from_environment(dir: &str, key: &str) -> Result<Self> {
        let mut files = Self::open(
            std::env::var(dir).map_err(bad)?,
            &hex::decode(std::env::var(key).map_err(bad)?).map_err(bad)?,
        )?;
        if dir == paydirt::environment::PAYDIRT_ARCHIVE_DIR {
            files = files.with_environment_notary()?;
        }
        Ok(files)
    }
    pub fn verification_from_environment() -> Result<Option<Self>> {
        use paydirt::environment::{PAYDIRT_ARCHIVE_DIR, PAYDIRT_ARCHIVE_KEY};
        let Some(root) = std::env::var_os(PAYDIRT_ARCHIVE_DIR) else {
            return Ok(None);
        };
        if std::env::var_os(PAYDIRT_ARCHIVE_KEY).is_some() {
            return Self::from_environment(PAYDIRT_ARCHIVE_DIR, PAYDIRT_ARCHIVE_KEY).map(Some);
        }
        let files = Self::without_decryption_key(root).with_environment_notary()?;
        if files.notary.is_none() {
            return Err(bad(
                "signed evidence or an archive decryption key is required",
            ));
        }
        Ok(Some(files))
    }
    fn with_environment_notary(mut self) -> Result<Self> {
        let signer: Option<Box<dyn Signer>> = match (
            std::env::var_os(paydirt::environment::PAYDIRT_SIGNING_KEY),
            std::env::var_os(paydirt::environment::PAYDIRT_SIGNER_PROGRAM),
        ) {
            (Some(path), None) => Some(Box::new(Ed25519::from_pkcs8(
                &fs::read(path).map_err(bad)?,
            )?)),
            (None, Some(program)) => Some(Box::new(ProgramSigner {
                executable: program.into(),
                arguments: serde_json::from_str(
                    &std::env::var(paydirt::environment::PAYDIRT_SIGNER_ARGS)
                        .unwrap_or_else(|_| "[]".into()),
                )
                .map_err(bad)?,
                public_key_der: fs::read(
                    std::env::var(paydirt::environment::PAYDIRT_SIGNER_PUBLIC_KEY).map_err(bad)?,
                )
                .map_err(bad)?,
            })),
            (Some(_), Some(_)) => return Err(bad("select a single signing provider")),
            (None, None) => None,
        };
        if let Some(signer) = signer {
            let mut witnesses: Vec<Box<dyn Witness>> = Vec::new();
            if let Some(path) = std::env::var_os(paydirt::environment::PAYDIRT_WITNESS_DIR) {
                witnesses.push(Box::new(DirectoryWitness(path.into())));
            }
            if let Ok(raw) = std::env::var(paydirt::environment::PAYDIRT_WITNESSES) {
                for spec in serde_json::from_str::<Vec<String>>(&raw).map_err(bad)? {
                    if spec == "log:" {
                        witnesses.push(Box::new(LogWitness));
                    } else if let Some(path) = spec.strip_prefix("store:").filter(|p| !p.is_empty())
                    {
                        witnesses.push(Box::new(DirectoryWitness(path.into())));
                    } else {
                        witnesses.push(Box::new(crate::rekor::Rekor::open(&spec)?));
                    }
                }
            }
            if witnesses.is_empty() {
                return Err(bad("missing external witness configuration"));
            }
            let trust: Vec<PathBuf> = serde_json::from_str(
                &std::env::var(paydirt::environment::PAYDIRT_TRUST_KEYS)
                    .unwrap_or_else(|_| "[]".into()),
            )
            .map_err(bad)?;
            let trust = trust
                .into_iter()
                .map(|p| fs::read(p).map_err(bad))
                .collect::<Result<_>>()?;
            self = self.with_notary(Notary::new(signer, trust, Box::new(Witnesses(witnesses))));
        }
        Ok(self)
    }
}

/// A deployment may isolate a KMS or device credential in its own local signer.
/// Stdin is the unmodified statement. Stdout is the raw signature.
pub struct ProgramSigner {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub public_key_der: Vec<u8>,
}
impl Signer for ProgramSigner {
    fn public_der(&self) -> Vec<u8> {
        self.public_key_der.clone()
    }
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>> {
        use std::process::{Command, Stdio};
        let mut child = Command::new(&self.executable)
            .args(&self.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(bad)?;
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| bad("signer stdin unavailable"))?;
        if let Err(e) = input.write_all(message) {
            drop(input);
            let _ = child.kill();
            let _ = child.wait();
            return Err(bad(e));
        }
        drop(input);
        let output = child.wait_with_output().map_err(bad)?;
        if !output.status.success() || output.stdout.is_empty() || output.stdout.len() > 8192 {
            return Err(bad("external signer failed"));
        }
        Ok(output.stdout)
    }
}
