//! Rekor v1 and v2 witnesses for deployment checkpoints.
mod tree;
use crate::{
    Fault, Result,
    attestation::{Claim, Signer, Witness, files, write_once},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::signature;
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use tree::Hash;
use url::Url;

fn bad(reason: impl ToString) -> Fault {
    Fault::precondition(format!("Rekor: {}", reason.to_string()))
}
fn string(v: &Value) -> Result<&str> {
    v.as_str()
        .ok_or_else(|| bad("expected a string in log response"))
}
fn number(v: &Value) -> Result<u64> {
    v.as_u64()
        .or_else(|| v.as_str()?.parse().ok())
        .ok_or_else(|| bad("invalid unsigned log index"))
}
fn b64(v: &Value) -> Result<Vec<u8>> {
    STANDARD.decode(string(v)?).map_err(bad)
}
fn hashes(v: &Value, base64: bool) -> Result<Vec<Hash>> {
    let Some(values) = v.as_array() else {
        if v.is_null() {
            return Ok(vec![]);
        }
        return Err(bad("invalid proof hashes"));
    };
    values.iter().map(|v| hash(v, base64)).collect()
}
fn hash(v: &Value, base64: bool) -> Result<Hash> {
    let raw = if base64 {
        b64(v)?
    } else {
        hex::decode(string(v)?).map_err(bad)?
    };
    raw.try_into().map_err(|_| bad("incorrect SHA-256 length"))
}
fn public_pem(key: &[u8]) -> String {
    let cfg = pem::EncodeConfig::new().set_line_ending(pem::LineEnding::LF);
    STANDARD.encode(pem::encode_config(&pem::Pem::new("PUBLIC KEY", key), cfg))
}
fn der(raw: &[u8]) -> Result<Vec<u8>> {
    if raw.starts_with(b"-----BEGIN ") {
        Ok(pem::parse(raw).map_err(bad)?.into_contents())
    } else {
        Ok(raw.to_vec())
    }
}
struct Public<'a> {
    bits: &'a [u8],
    kind: &'static str,
    hash: &'static str,
    alg: &'static dyn signature::VerificationAlgorithm,
}
fn public(raw: &[u8]) -> Result<Public<'_>> {
    let key = spki::SubjectPublicKeyInfoRef::try_from(raw).map_err(bad)?;
    let bits = key
        .subject_public_key
        .as_bytes()
        .ok_or_else(|| bad("invalid public key"))?;
    let (kind, hash, alg): (_, _, &dyn signature::VerificationAlgorithm) =
        match key.algorithm.oid.to_string().as_str() {
            "1.3.101.112" => ("PKIX_ED25519_PH", "sha512", &signature::ED25519),
            "1.2.840.10045.2.1" => match key
                .algorithm
                .parameters_oid()
                .map_err(bad)?
                .to_string()
                .as_str()
            {
                "1.2.840.10045.3.1.7" => (
                    "PKIX_ECDSA_P256_SHA_256",
                    "sha256",
                    &signature::ECDSA_P256_SHA256_ASN1,
                ),
                "1.3.132.0.34" => (
                    "PKIX_ECDSA_P384_SHA_384",
                    "sha384",
                    &signature::ECDSA_P384_SHA384_ASN1,
                ),
                _ => return Err(bad("unsupported ECDSA curve")),
            },
            "1.2.840.113549.1.1.1" => {
                use pkcs1::der::Decode;
                let width = pkcs1::RsaPublicKey::from_der(bits)
                    .map_err(bad)?
                    .modulus
                    .as_bytes()
                    .len();
                let kind = match width {
                    256 => "PKIX_RSA_PKCS1V15_2048_SHA256",
                    384 => "PKIX_RSA_PKCS1V15_3072_SHA256",
                    512 => "PKIX_RSA_PKCS1V15_4096_SHA256",
                    _ => return Err(bad("unsupported RSA size")),
                };
                (kind, "sha256", &signature::RSA_PKCS1_2048_8192_SHA256)
            }
            _ => return Err(bad("unsupported public key algorithm")),
        };
    Ok(Public {
        bits,
        kind,
        hash,
        alg,
    })
}
fn digest(alg: &str, doc: &[u8]) -> Result<Vec<u8>> {
    match alg {
        "sha256" => Ok(Sha256::digest(doc).to_vec()),
        "sha384" => Ok(sha2::Sha384::digest(doc).to_vec()),
        "sha512" => Ok(Sha512::digest(doc).to_vec()),
        _ => Err(bad("unsupported hash")),
    }
}
struct SignedTree {
    origin: String,
    size: u64,
    root: Hash,
}
pub struct Rekor {
    base: Url,
    v2: bool,
    origin: String,
    key: Vec<u8>,
    receipt_dir: String,
    http: ureq::Agent,
}
impl Rekor {
    pub fn open(spec: &str) -> Result<Self> {
        let mut base = Url::parse(
            spec.strip_prefix("rekor:")
                .ok_or_else(|| bad("unknown witness scheme"))?,
        )
        .map_err(bad)?;
        if !matches!(base.scheme(), "https" | "http")
            || !base.has_host()
            || base.fragment().is_some()
            || !base.username().is_empty()
            || base.password().is_some()
        {
            return Err(bad("invalid log URL"));
        }
        let options: BTreeMap<_, _> = base.query_pairs().into_owned().collect();
        base.set_query(None);
        let key = der(&fs::read(
            options
                .get("key")
                .ok_or_else(|| bad("log public key is required"))?,
        )
        .map_err(bad)?)?;
        if public(&key)?.kind.starts_with("PKIX_RSA") {
            return Err(bad("log note requires ECDSA or Ed25519"));
        }
        let origin = options
            .get("origin")
            .cloned()
            .unwrap_or_else(|| base[url::Position::BeforeHost..url::Position::AfterPort].into());
        let v2 = match options.get("v").map(String::as_str) {
            None | Some("1") => false,
            Some("2") => true,
            _ => return Err(bad("unsupported log version")),
        };
        let receipt_dir = hex::encode(
            Sha256::new()
                .chain_update(base.as_str())
                .chain_update([v2 as u8])
                .chain_update(&origin)
                .chain_update(&key)
                .finalize(),
        );
        Ok(Self {
            base,
            v2,
            origin,
            key,
            receipt_dir,
            http: ureq::AgentBuilder::new()
                .redirects(0)
                .timeout(Duration::from_secs(30))
                .build(),
        })
    }
    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.base.as_str().trim_end_matches('/'), path)
    }
    fn response(r: std::result::Result<ureq::Response, ureq::Error>) -> Result<ureq::Response> {
        match r {
            Ok(v) | Err(ureq::Error::Status(_, v)) => Ok(v),
            Err(e) => Err(bad(e)),
        }
    }
    fn get(&self, path: &str) -> Result<ureq::Response> {
        Self::response(self.http.get(&self.endpoint(path)).call())
    }
    fn post(&self, path: &str, data: &Value) -> Result<ureq::Response> {
        Self::response(self.http.post(&self.endpoint(path)).send_json(data))
    }
    fn body(&self, r: ureq::Response) -> Result<Vec<u8>> {
        if !(200..300).contains(&r.status()) {
            return Err(bad(format!("log returned HTTP {}", r.status())));
        }
        let mut bytes = Vec::new();
        r.into_reader()
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(bad)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(bad("oversized log response"));
        }
        Ok(bytes)
    }
    fn json(&self, r: ureq::Response) -> Result<Value> {
        serde_json::from_slice(&self.body(r)?).map_err(bad)
    }
    fn tree(&self, note: &str) -> Result<SignedTree> {
        let (text, signature_lines) = note
            .split_once("\n\n")
            .ok_or_else(|| bad("unsigned tree head"))?;
        let mut lines = text.lines();
        let origin = lines.next().ok_or_else(|| bad("missing tree origin"))?;
        if origin != self.origin
            && (self.v2 || origin.split_once(" - ").map(|v| v.0) != Some(&self.origin))
        {
            return Err(bad("unexpected tree origin"));
        }
        let size = lines
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| bad("invalid tree size"))?;
        let root = STANDARD
            .decode(lines.next().ok_or_else(|| bad("missing tree root"))?)
            .map_err(bad)?
            .try_into()
            .map_err(|_| bad("invalid tree root"))?;
        let key = public(&self.key)?;
        let id = if key.kind == "PKIX_ED25519_PH" {
            Sha256::new()
                .chain_update(&self.origin)
                .chain_update([10, 1])
                .chain_update(key.bits)
                .finalize()
        } else {
            Sha256::digest(&self.key)
        };
        let mut message = text.as_bytes().to_vec();
        message.push(b'\n');
        let signed = signature_lines
            .lines()
            .filter_map(|line| line.strip_prefix("— ")?.split_once(' '))
            .any(|(name, s)| {
                if name != self.origin {
                    return false;
                }
                STANDARD.decode(s).ok().is_some_and(|bytes| {
                    bytes.starts_with(&id[..4])
                        && signature::UnparsedPublicKey::new(key.alg, key.bits)
                            .verify(&message, &bytes[4..])
                            .is_ok()
                })
            });
        if !signed {
            return Err(bad("tree head signature mismatch"));
        }
        Ok(SignedTree {
            origin: origin.into(),
            size,
            root,
        })
    }
    fn fresh(&self) -> Result<SignedTree> {
        if self.v2 {
            let data = self.body(self.get("/checkpoint")?)?;
            self.tree(std::str::from_utf8(&data).map_err(bad)?)
        } else {
            self.tree(string(
                &self.json(self.get("/api/v1/log")?)?["signedTreeHead"],
            )?)
        }
    }
    fn proof(&self, entry: &Value) -> Result<(Vec<u8>, SignedTree)> {
        let (body, p, note) = if self.v2 {
            (
                &entry["canonicalizedBody"],
                &entry["inclusionProof"],
                &entry["inclusionProof"]["checkpoint"]["envelope"],
            )
        } else {
            (
                &entry["body"],
                &entry["verification"]["inclusionProof"],
                &entry["verification"]["inclusionProof"]["checkpoint"],
            )
        };
        let body = b64(body)?;
        let index = number(&p["logIndex"])?;
        let size = number(&p["treeSize"])?;
        let root = hash(&p["rootHash"], self.v2)?;
        if index != number(&entry["logIndex"])? {
            return Err(bad("entry and proof index differ"));
        }
        tree::inclusion(
            index,
            size,
            tree::leaf(&body),
            &hashes(&p["hashes"], self.v2)?,
            root,
        )?;
        let head = self.tree(string(note)?)?;
        if head.size != size || head.root != root {
            return Err(bad("signed head differs from entry proof"));
        }
        Ok((body, head))
    }
    fn chain(&self, a: &SignedTree, b: &SignedTree) -> Result<()> {
        if a.origin != b.origin {
            return Ok(());
        }
        if a.size == b.size && a.root == b.root {
            return Ok(());
        }
        if a.size >= b.size {
            return Err(bad("log rewrote a signed tree"));
        }
        if self.v2 {
            return tree::tiled_consistency(self, a.size, b.size, a.root, b.root);
        }
        let mut q = url::form_urlencoded::Serializer::new(String::new());
        q.append_pair("firstSize", &a.size.to_string())
            .append_pair("lastSize", &b.size.to_string());
        if let Some((_, id)) = a.origin.split_once(" - ") {
            q.append_pair("treeID", id);
        }
        let rsp = self.json(self.get(&format!("/api/v1/log/proof?{}", q.finish()))?)?;
        tree::consistency(
            a.size,
            b.size,
            a.root,
            b.root,
            &hashes(&rsp["hashes"], false)?,
        )
    }
    fn receipt_path(&self, root: &Path) -> PathBuf {
        root.join(".receipts").join(&self.receipt_dir)
    }
}

struct Record {
    key: Vec<u8>,
    algorithm: String,
    digest: Vec<u8>,
    signature: Vec<u8>,
}
impl Record {
    fn read(body: &[u8], v2: bool) -> Result<Self> {
        let v: Value = serde_json::from_slice(body).map_err(bad)?;
        if v["kind"] != "hashedrekord" || v["apiVersion"] != if v2 { "0.0.2" } else { "0.0.1" } {
            return Err(bad("unexpected log entry type"));
        }
        if v2 {
            let r = &v["spec"]["hashedRekordV002"];
            let s = &r["signature"];
            let key = b64(&s["verifier"]["publicKey"]["rawBytes"])?;
            let public = public(&key)?;
            let algorithm = public.hash.to_owned();
            if s["verifier"]["keyDetails"] != public.kind
                || r["data"]["algorithm"] != format!("SHA2_{}", &algorithm[3..])
            {
                return Err(bad("digest and key algorithms disagree"));
            }
            Ok(Self {
                key,
                algorithm,
                digest: b64(&r["data"]["digest"])?,
                signature: b64(&s["content"])?,
            })
        } else {
            let r = &v["spec"];
            Ok(Self {
                key: der(&b64(&r["signature"]["publicKey"]["content"])?)?,
                algorithm: string(&r["data"]["hash"]["algorithm"])?.into(),
                digest: hex::decode(string(&r["data"]["hash"]["value"])?).map_err(bad)?,
                signature: b64(&r["signature"]["content"])?,
            })
        }
    }
    fn check(&self, claim: &Claim, trusted: &BTreeSet<Vec<u8>>) -> Result<()> {
        let doc = claim.statement.as_bytes();
        if !trusted.contains(&self.key) || digest(&self.algorithm, doc)? != self.digest {
            return Err(bad("entry does not bind this checkpoint and trusted key"));
        }
        let key = public(&self.key)?;
        if key.kind == "PKIX_ED25519_PH" {
            if self.algorithm != "sha512" {
                return Err(bad("Ed25519ph requires SHA-512"));
            }
            let public = ed25519_dalek::VerifyingKey::try_from(key.bits).map_err(bad)?;
            public
                .verify_prehashed_strict(
                    Sha512::new_with_prefix(doc),
                    None,
                    &ed25519_dalek::Signature::from_slice(&self.signature).map_err(bad)?,
                )
                .map_err(bad)
        } else {
            let alg = if key.hash == "sha384" && self.algorithm == "sha256" {
                &signature::ECDSA_P384_SHA256_ASN1
            } else {
                key.alg
            };
            signature::UnparsedPublicKey::new(alg, key.bits)
                .verify(doc, &self.signature)
                .map_err(bad)
        }
    }
}
impl Rekor {
    fn add(&self, claim: &Claim, signer: &dyn Signer) -> Result<Value> {
        let key = signer.public_der();
        let params = public(&key)?;
        let doc = claim.statement.as_bytes();
        let algorithm = if !self.v2 && params.hash == "sha384" {
            "sha256"
        } else {
            params.hash
        };
        let sum = digest(algorithm, doc)?;
        let sig = if params.kind == "PKIX_ED25519_PH" {
            signer.sign_prehash(doc)?
        } else if claim.signature.key == STANDARD.encode(&key) {
            STANDARD.decode(&claim.signature.sig).map_err(bad)?
        } else {
            signer.sign(doc)?
        };
        if self.v2 {
            let req = json!({"hashedRekordRequestV002":{"digest":STANDARD.encode(sum),"signature":{"content":STANDARD.encode(sig),"verifier":{"publicKey":{"rawBytes":STANDARD.encode(&key)},"keyDetails":params.kind}}}});
            return self.json(self.post("/api/v2/log/entries", &req)?);
        }
        let req = json!({"apiVersion":"0.0.1","kind":"hashedrekord","spec":{"data":{"hash":{"algorithm":algorithm,"value":hex::encode(sum)}},"signature":{"content":STANDARD.encode(sig),"publicKey":{"content":public_pem(&key)}}}});
        let mut response = self.post("/api/v1/log/entries", &req)?;
        if response.status() == 409 {
            let location = response
                .header("Location")
                .ok_or_else(|| bad("duplicate entry without location"))?;
            let url = self.base.join(location).map_err(bad)?;
            if url.origin() != self.base.origin() || !url.path().starts_with("/api/v1/log/entries/")
            {
                return Err(bad("unexpected duplicate entry location"));
            }
            response = Self::response(self.http.get(url.as_str()).call())?;
        }
        let entries = self.json(response)?;
        let entries = entries
            .as_object()
            .filter(|v| v.len() == 1)
            .ok_or_else(|| bad("expected one log entry"))?;
        Ok(entries.values().next().unwrap().clone())
    }
    fn check_search(
        &self,
        history: &BTreeMap<u64, Claim>,
        trusted: &BTreeSet<Vec<u8>>,
    ) -> Result<()> {
        let mut ours = BTreeSet::new();
        for claim in history.values() {
            for alg in ["sha256", "sha384", "sha512"] {
                ours.insert(digest(alg, claim.statement.as_bytes())?);
            }
        }
        let mut ids = BTreeSet::new();
        for key in trusted {
            let req = json!({"publicKey":{"format":"x509","content":public_pem(key)}});
            let found = self.json(self.post("/api/v1/index/retrieve", &req)?)?;
            if !found.is_null() {
                for id in found
                    .as_array()
                    .ok_or_else(|| bad("invalid search response"))?
                {
                    ids.insert(string(id)?.to_owned());
                }
            }
        }
        let ids = ids.into_iter().collect::<Vec<_>>();
        for batch in ids.chunks(10) {
            let response = self
                .json(self.post("/api/v1/log/entries/retrieve", &json!({"entryUUIDs":batch}))?)?;
            let mut seen = BTreeSet::new();
            for group in response
                .as_array()
                .ok_or_else(|| bad("invalid entry lookup"))?
            {
                for (uuid, entry) in group
                    .as_object()
                    .ok_or_else(|| bad("invalid entry lookup result"))?
                {
                    let (body, _) = self.proof(entry)?;
                    let parsed: Value = serde_json::from_slice(&body).map_err(bad)?;
                    if parsed["kind"] == "hashedrekord" {
                        let record = Record::read(&body, false)?;
                        if trusted.contains(&record.key) && !ours.contains(&record.digest) {
                            return Err(bad(
                                "unkept: the log contains a signed entry for no checkpoint the store keeps",
                            ));
                        }
                    }
                    seen.insert(uuid);
                }
            }
            if batch.iter().any(|id| !seen.contains(id)) {
                return Err(bad("log omitted a searched entry"));
            }
        }
        Ok(())
    }
}
impl Witness for Rekor {
    fn submit(&self, root: &Path, claim: &Claim, signer: &dyn Signer) -> Result<()> {
        let entry = self.add(claim, signer)?;
        let (body, _) = self.proof(&entry)?;
        Record::read(&body, self.v2)?.check(claim, &BTreeSet::from([signer.public_der()]))?;
        write_once(
            &self
                .receipt_path(root)
                .join(format!("{:020}.json", claim.head.k)),
            &serde_json::to_vec(&entry).map_err(bad)?,
        )
    }
    fn check(
        &self,
        root: &Path,
        history: &BTreeMap<u64, Claim>,
        signer: &dyn Signer,
        trusted: &BTreeSet<Vec<u8>>,
        advance: bool,
    ) -> Result<()> {
        let dir = self.receipt_path(root);
        let receipts = files(&dir, "json")?;
        if receipts.keys().any(|k| !history.contains_key(k)) {
            return Err(bad("unkept checkpoint named by a receipt"));
        }
        let mut heads = Vec::new();
        for (k, claim) in history {
            let path = dir.join(format!("{k:020}.json"));
            if !receipts.contains_key(k) {
                if !advance {
                    return Err(bad("missing checkpoint receipt"));
                }
                self.submit(root, claim, signer)?;
            }
            let entry = serde_json::from_slice(&fs::read(path).map_err(bad)?).map_err(bad)?;
            let (body, head) = self.proof(&entry)?;
            Record::read(&body, self.v2)?.check(claim, trusted)?;
            heads.push(head);
        }
        heads.sort_by(|a, b| (&a.origin, a.size).cmp(&(&b.origin, b.size)));
        for pair in heads.windows(2) {
            self.chain(&pair[0], &pair[1])?;
        }
        let fresh = self.fresh()?;
        if let Some(prior) = heads.iter().rev().find(|h| h.origin == fresh.origin) {
            self.chain(prior, &fresh)?;
        }
        if !self.v2 {
            self.check_search(history, trusted)?;
        }
        Ok(())
    }
}
