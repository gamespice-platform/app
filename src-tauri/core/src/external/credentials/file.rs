//! An encrypted file as a [`SecretBackend`], for machines with no keychain.
//!
//! The file is a JSON map of account to sealed value ([`LocalCipher`], XChaCha20
//! -Poly1305). The key comes from a [`KeySource`]: a user passphrase, or a
//! machine-bound secret (a random key file next to the data, mixed with the
//! OS machine id where there is one, so the file does not decrypt if copied to
//! another computer without its key file).
//!
//! This is weaker than a keychain, which is why the store reports "fallback
//! mode" whenever it is in use.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rand::Rng;
use sha2::{Digest, Sha256};

use super::{BackendKind, Secret, SecretBackend};
use crate::crypto::LocalCipher;
use crate::error::{AppError, AppResult};

const DOMAIN: &[u8] = b"gamespice-app external credentials v1";
const PASSPHRASE_ROUNDS: u32 = 200_000;

/// Where the file's encryption key comes from.
pub enum KeySource {
    /// A key derived from what the user typed.
    Passphrase(String),
    /// A random key file beside the data, bound to this machine's id.
    Machine { key_file: PathBuf },
}

impl KeySource {
    pub fn machine(data_dir: &Path) -> Self {
        Self::Machine {
            key_file: data_dir.join("external-credentials.key"),
        }
    }

    fn derive(&self) -> AppResult<[u8; 32]> {
        match self {
            Self::Passphrase(p) => {
                if p.is_empty() {
                    return Err(AppError::invalid("The passphrase is empty."));
                }
                // Iterated, salted hash: slows guessing without a new
                // dependency. A real KDF is a known follow-up.
                let mut h = Sha256::new();
                h.update(DOMAIN);
                h.update(p.as_bytes());
                let mut out: [u8; 32] = h.finalize().into();
                for _ in 0..PASSPHRASE_ROUNDS {
                    let mut h = Sha256::new();
                    h.update(DOMAIN);
                    h.update(out);
                    h.update(p.as_bytes());
                    out = h.finalize().into();
                }
                Ok(out)
            }
            Self::Machine { key_file } => {
                let random = load_or_create_key_file(key_file)?;
                let mut h = Sha256::new();
                h.update(DOMAIN);
                h.update(random);
                if let Some(id) = machine_id() {
                    h.update(id.as_bytes());
                }
                Ok(h.finalize().into())
            }
        }
    }
}

fn machine_id() -> Option<String> {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn load_or_create_key_file(path: &Path) -> AppResult<[u8; 32]> {
    match std::fs::read(path) {
        Ok(bytes) => <[u8; 32]>::try_from(bytes.as_slice())
            .map_err(|_| AppError::internal("the credential key file is damaged")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut key = [0u8; 32];
            rand::rng().fill_bytes(&mut key);
            write_private(path, &key)?;
            Ok(key)
        }
        Err(e) => Err(AppError::internal(format!("credential key file: {e}"))),
    }
}

/// Write via a temp file and rename, owner-only on unix.
fn write_private(path: &Path, bytes: &[u8]) -> AppResult<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| AppError::internal(format!("credential folder: {e}")))?;
    }
    let tmp = path.with_extension("tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(&tmp)
        .map_err(|e| AppError::internal(format!("credential file: {e}")))?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| AppError::internal(format!("credential file: {e}")))?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(|e| AppError::internal(format!("credential file: {e}")))
}

pub struct EncryptedFile {
    path: PathBuf,
    source: KeySource,
    /// Serialises read-modify-write.
    lock: Mutex<()>,
}

impl EncryptedFile {
    pub fn new(path: PathBuf, source: KeySource) -> Self {
        Self {
            path,
            source,
            lock: Mutex::new(()),
        }
    }

    fn cipher(&self) -> AppResult<LocalCipher> {
        Ok(LocalCipher::from_key(&self.source.derive()?))
    }

    fn read(&self) -> AppResult<BTreeMap<String, String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|_| AppError::internal("the credential file is damaged")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(AppError::internal(format!("credential file: {e}"))),
        }
    }

    fn write(&self, map: &BTreeMap<String, String>) -> AppResult<()> {
        let text = serde_json::to_vec(map)
            .map_err(|e| AppError::internal(format!("credential file: {e}")))?;
        write_private(&self.path, &text)
    }
}

impl SecretBackend for EncryptedFile {
    fn kind(&self) -> BackendKind {
        BackendKind::EncryptedFile
    }

    fn get(&self, account: &str) -> AppResult<Option<Secret>> {
        let _g = self
            .lock
            .lock()
            .map_err(|_| AppError::internal("credential file lock poisoned"))?;
        let Some(sealed) = self.read()?.get(account).cloned() else {
            return Ok(None);
        };
        // A value that will not open is an error, not "no key": the caller
        // must not treat a wrong passphrase as an empty one.
        Ok(Some(Secret::new(self.cipher()?.open(&sealed)?)))
    }

    fn set(&self, account: &str, value: &Secret) -> AppResult<()> {
        let _g = self
            .lock
            .lock()
            .map_err(|_| AppError::internal("credential file lock poisoned"))?;
        let mut map = self.read()?;
        map.insert(account.to_string(), self.cipher()?.seal(value.expose())?);
        self.write(&map)
    }

    fn delete(&self, account: &str) -> AppResult<()> {
        let _g = self
            .lock
            .lock()
            .map_err(|_| AppError::internal("credential file lock poisoned"))?;
        let mut map = self.read()?;
        if map.remove(account).is_some() {
            self.write(&map)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(dir: &Path) -> EncryptedFile {
        EncryptedFile::new(dir.join("c.enc"), KeySource::machine(dir))
    }

    #[test]
    fn round_trip_and_the_file_holds_no_plaintext() {
        let d = tempfile::tempdir().unwrap();
        let f = machine(d.path());
        f.set("a", &Secret::new("plainsecret-xyz")).unwrap();
        assert_eq!(f.get("a").unwrap().unwrap().expose(), "plainsecret-xyz");
        let raw = std::fs::read_to_string(d.path().join("c.enc")).unwrap();
        assert!(!raw.contains("plainsecret-xyz"));
        // A new instance (a restart) reads it back.
        assert_eq!(
            machine(d.path()).get("a").unwrap().unwrap().expose(),
            "plainsecret-xyz"
        );
        f.delete("a").unwrap();
        assert!(f.get("a").unwrap().is_none());
        f.delete("a").unwrap();
    }

    #[test]
    fn wrong_passphrase_is_an_error_not_an_empty_key() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("p.enc");
        let right = EncryptedFile::new(p.clone(), KeySource::Passphrase("right".into()));
        right.set("a", &Secret::new("v")).unwrap();
        let wrong = EncryptedFile::new(p, KeySource::Passphrase("wrong".into()));
        assert!(wrong.get("a").is_err());
        assert_eq!(right.get("a").unwrap().unwrap().expose(), "v");
    }

    #[test]
    fn damaged_file_and_key_are_errors() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("c.enc"), "not json").unwrap();
        assert!(machine(d.path()).get("a").is_err());
        let d2 = tempfile::tempdir().unwrap();
        std::fs::write(d2.path().join("external-credentials.key"), [1u8; 5]).unwrap();
        assert!(machine(d2.path()).set("a", &Secret::new("v")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let f = machine(d.path());
        f.set("a", &Secret::new("v")).unwrap();
        for name in ["c.enc", "external-credentials.key"] {
            let m = std::fs::metadata(d.path().join(name)).unwrap();
            assert_eq!(m.permissions().mode() & 0o077, 0, "{name}");
        }
    }
}
