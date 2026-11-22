//! Immutable encrypted archive objects and authenticated reads. Manifests contain only digests.
use crate::{Fault, Result};
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
const MAGIC: &[u8] = b"paydirt.archive.v1\0";
const LIMIT: u64 = 64 * 1024 * 1024;
fn io(e: std::io::Error) -> Fault {
    Fault::unavailable(e.to_string())
}
pub struct Archive {
    pub(crate) root: PathBuf,
    pub(crate) notary: Option<crate::attestation::Notary>,
    key: Option<aead::LessSafeKey>,
}
impl Archive {
    pub fn open(root: impl AsRef<Path>, key: &[u8]) -> Result<Self> {
        let key = aead::UnboundKey::new(&aead::AES_256_GCM, key)
            .map_err(|_| Fault::invalid("expected a 32-byte archive key"))?;
        Ok(Self {
            root: root.as_ref().into(),
            notary: None,
            key: Some(aead::LessSafeKey::new(key)),
        })
    }
    pub fn without_decryption_key(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().into(),
            notary: None,
            key: None,
        }
    }
    fn key(&self) -> Result<&aead::LessSafeKey> {
        self.key
            .as_ref()
            .ok_or_else(|| Fault::precondition("archive decryption key required"))
    }
    pub fn digest(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }
    fn path(&self, digest: &str) -> Result<PathBuf> {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Fault::invalid("invalid archive digest"));
        }
        Ok(self.root.join(digest))
    }
    fn sync_dir(&self) -> Result<()> {
        File::open(&self.root)
            .and_then(|f| f.sync_all())
            .map_err(io)
    }
    fn ciphertext(&self, digest: &str) -> Result<Vec<u8>> {
        let mut wire = Vec::new();
        File::open(self.path(digest)?)
            .map_err(io)?
            .take(LIMIT + 1)
            .read_to_end(&mut wire)
            .map_err(io)?;
        if wire.len() as u64 > LIMIT || !wire.starts_with(MAGIC) || wire.len() < MAGIC.len() + 28 {
            return Err(Fault::precondition("invalid archive object"));
        }
        Ok(wire)
    }
    pub(crate) fn ciphertext_digest(&self, object: &str) -> Result<String> {
        Ok(Self::digest(&self.ciphertext(object)?))
    }
    pub fn read(&self, digest: &str) -> Result<Vec<u8>> {
        let key = self.key()?;
        let mut wire = self.ciphertext(digest)?;
        let (prefix, cipher) = wire.split_at_mut(MAGIC.len() + 12);
        let nonce = prefix[MAGIC.len()..].try_into().unwrap();
        let payload = key
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(MAGIC),
                cipher,
            )
            .map_err(|_| Fault::precondition("archive authentication failed"))?;
        if Self::digest(payload) != digest {
            return Err(Fault::precondition("archive content digest mismatch"));
        }
        Ok(payload.to_vec())
    }
    pub fn put(&self, data: &[u8]) -> Result<String> {
        let key = self.key()?;
        if data.len() as u64 > LIMIT - 128 {
            return Err(Fault::exhausted("archive object exceeds 64 MiB"));
        }
        let digest = Self::digest(data);
        let path = self.path(&digest)?;
        fs::create_dir_all(&self.root).map_err(io)?;
        if path.try_exists().map_err(io)? {
            self.read(&digest)?;
            return Ok(digest);
        }
        let mut nonce = [0; 12];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| Fault::internal("archive randomness unavailable"))?;
        let tmp = self.root.join(format!(".tmp-{}", hex::encode(nonce)));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options.open(&tmp).map_err(io)?;
        let result = (|| {
            let mut cipher = data.to_vec();
            key.seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(MAGIC),
                &mut cipher,
            )
            .map_err(|_| Fault::internal("archive encryption failed"))?;
            f.write_all(MAGIC)
                .and_then(|_| f.write_all(&nonce))
                .and_then(|_| f.write_all(&cipher))
                .and_then(|_| f.sync_all())
                .map_err(io)?;
            match fs::hard_link(&tmp, &path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    self.read(&digest)?;
                }
                Err(e) => return Err(io(e)),
            }
            self.sync_dir()
        })();
        drop(f);
        let _ = fs::remove_file(tmp);
        result?;
        Ok(digest)
    }
    pub(crate) fn stage(&self, data: &[u8]) -> Result<String> {
        fs::create_dir_all(&self.root).map_err(io)?;
        let id = Self::digest(data);
        let marker = self.root.join(format!(".pending-{id}"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
        {
            Ok(f) => {
                f.sync_all().map_err(io)?;
                self.sync_dir()?
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io(e)),
        }
        self.put(data)
    }
    pub(crate) fn recover(&self, active: &std::collections::BTreeSet<String>) -> Result<()> {
        let entries = match fs::read_dir(&self.root) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(io(e)),
        };
        for item in entries {
            let name = item.map_err(io)?.file_name();
            if let Some(id) = name.to_str().and_then(|n| n.strip_prefix(".pending-")) {
                self.path(id)?;
                if !active.contains(id) {
                    self.remove(id)?;
                }
                fs::remove_file(self.root.join(name)).map_err(io)?;
            }
        }
        self.sync_dir()
    }
    pub(crate) fn remove(&self, digest: &str) -> Result<()> {
        match fs::remove_file(self.path(digest)?) {
            Ok(()) => self.sync_dir(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io(e)),
        }
    }
}
