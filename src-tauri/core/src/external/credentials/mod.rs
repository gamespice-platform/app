//! The user's own credentials for external providers (a Nexus API key).
//!
//! Two kinds of key exist and they are kept apart on purpose:
//!
//! * **Ours** (CurseForge, mod.io) live in the TMC AdminCP. The site proxies the
//!   call ([`super::site`]); the app never holds them.
//! * **The user's** (a Nexus key) are held here, in the strongest store the
//!   machine has, and never leave it except in the one request header that
//!   needs them.
//!
//! Storage reuses [`crate::secure`]'s naming and the OS credential store it
//! already uses for the sign-in token, but not its silent plaintext fallback:
//! when there is no keychain this store writes an *encrypted* file
//! ([`file::EncryptedFile`]) and says so ([`StoreMode::fallback`]), so the
//! settings screen can show "fallback mode" instead of implying a keychain.
//!
//! Secrets never appear in `Debug`, logs, settings JSON or IPC results:
//! [`Secret`] redacts itself and [`CredentialReport`] carries only
//! configured / not configured.

pub mod file;
#[cfg(all(
    feature = "os-keyring",
    any(target_os = "windows", target_os = "macos", target_os = "linux")
))]
pub mod keychain;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use super::provider::ProviderAvailability;
use super::ProviderId;
use crate::error::{AppError, AppResult};

/// Longest key accepted. Real ones are under 200 characters.
const MAX_SECRET_LEN: usize = 4096;

/// A secret that cannot be printed by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value, for the one place that must send it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Where the secrets actually sit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BackendKind {
    /// Windows Credential Manager, macOS Keychain, the Secret Service.
    Keychain,
    /// A file encrypted with a key from a passphrase or this machine.
    EncryptedFile,
    /// Nowhere durable (tests, or nothing available).
    Memory,
}

/// One place to keep named secrets. Implementations: the OS keychain, an
/// encrypted file, and an in-memory map for tests.
pub trait SecretBackend: Send + Sync {
    fn kind(&self) -> BackendKind;

    /// Can this backend be used right now? A Linux box with no Secret Service
    /// says no.
    fn available(&self) -> bool {
        true
    }

    fn get(&self, account: &str) -> AppResult<Option<Secret>>;
    fn set(&self, account: &str, value: &Secret) -> AppResult<()>;
    /// Deleting what is not there is success.
    fn delete(&self, account: &str) -> AppResult<()>;
}

/// A map, for tests. Never used in the app.
#[derive(Default)]
pub struct MemoryBackend {
    map: Mutex<BTreeMap<String, String>>,
    /// Pretend the backend is missing (a keychain that is not running).
    pub unavailable: bool,
}

impl MemoryBackend {
    pub fn unavailable() -> Self {
        Self {
            unavailable: true,
            ..Self::default()
        }
    }
}

impl SecretBackend for MemoryBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Memory
    }
    fn available(&self) -> bool {
        !self.unavailable
    }
    fn get(&self, account: &str) -> AppResult<Option<Secret>> {
        if self.unavailable {
            return Err(AppError::internal("the backend is not available"));
        }
        Ok(self
            .map
            .lock()
            .map_err(|_| AppError::internal("credential map poisoned"))?
            .get(account)
            .map(|v| Secret::new(v.clone())))
    }
    fn set(&self, account: &str, value: &Secret) -> AppResult<()> {
        if self.unavailable {
            return Err(AppError::internal("the backend is not available"));
        }
        self.map
            .lock()
            .map_err(|_| AppError::internal("credential map poisoned"))?
            .insert(account.to_string(), value.expose().to_string());
        Ok(())
    }
    fn delete(&self, account: &str) -> AppResult<()> {
        self.map
            .lock()
            .map_err(|_| AppError::internal("credential map poisoned"))?
            .remove(account);
        Ok(())
    }
}

/// What a provider asks to find the user's key.
pub trait CredentialLookup: Send + Sync {
    fn user_key(&self, provider: ProviderId) -> Option<Secret>;
}

/// No credentials at all.
pub struct NoCredentials;

impl CredentialLookup for NoCredentials {
    fn user_key(&self, _provider: ProviderId) -> Option<Secret> {
        None
    }
}

/// Providers the user may hold a key for in this build.
///
/// CurseForge and mod.io keys are ours (proxied by the site). A user's own
/// CurseForge or mod.io key is not wired to a direct lookup yet, so setting one
/// is refused rather than stored and then ignored.
pub fn accepts_user_key(provider: ProviderId) -> bool {
    matches!(provider, ProviderId::Nexus)
}

fn account(provider: ProviderId) -> String {
    format!("external-{}", provider.as_str())
}

/// Reject keys that cannot be real before they reach a keychain or a header.
fn validate(value: &str) -> AppResult<String> {
    let v = value.trim();
    if v.is_empty() {
        return Err(AppError::invalid("The key is empty."));
    }
    if v.len() > MAX_SECRET_LEN {
        return Err(AppError::invalid("That key is too long to be real."));
    }
    if v.chars().any(|c| c.is_control()) {
        return Err(AppError::invalid("The key contains control characters."));
    }
    Ok(v.to_string())
}

/// Where credentials are being kept, for the settings screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreMode {
    pub backend: BackendKind,
    /// True when the OS keychain is not what is in use.
    pub fallback: bool,
}

/// The store: the keychain when there is one, an encrypted file when not.
pub struct CredentialStore {
    primary: Option<Box<dyn SecretBackend>>,
    fallback: Box<dyn SecretBackend>,
}

impl CredentialStore {
    pub fn new(primary: Option<Box<dyn SecretBackend>>, fallback: Box<dyn SecretBackend>) -> Self {
        Self { primary, fallback }
    }

    /// The real thing: keychain (desktop builds with `os-keyring`), then an
    /// encrypted file in `data_dir` keyed by this machine.
    pub fn system(data_dir: &std::path::Path) -> Self {
        #[cfg(all(
            feature = "os-keyring",
            any(target_os = "windows", target_os = "macos", target_os = "linux")
        ))]
        let primary: Option<Box<dyn SecretBackend>> = Some(Box::new(keychain::Keychain::new()));
        #[cfg(not(all(
            feature = "os-keyring",
            any(target_os = "windows", target_os = "macos", target_os = "linux")
        )))]
        let primary: Option<Box<dyn SecretBackend>> = None;
        Self::new(
            primary,
            Box::new(file::EncryptedFile::new(
                data_dir.join("external-credentials.enc"),
                file::KeySource::machine(data_dir),
            )),
        )
    }

    fn usable_primary(&self) -> Option<&dyn SecretBackend> {
        self.primary.as_deref().filter(|p| p.available())
    }

    pub fn mode(&self) -> StoreMode {
        match self.usable_primary() {
            Some(p) => StoreMode {
                backend: p.kind(),
                fallback: false,
            },
            None => StoreMode {
                backend: self.fallback.kind(),
                fallback: true,
            },
        }
    }

    /// Keep `value` for `provider`. Returns where it went.
    pub fn set(&self, provider: ProviderId, value: &str) -> AppResult<StoreMode> {
        if !accepts_user_key(provider) {
            return Err(AppError::invalid(format!(
                "{} works through the TMC site; there is no key of your own to add for it yet.",
                provider.label()
            )));
        }
        let secret = Secret::new(validate(value)?);
        let acct = account(provider);
        if let Some(p) = self.usable_primary() {
            // Read it back: a keychain that accepts a write and returns
            // nothing (a locked collection) must not count as stored.
            let stored = p
                .set(&acct, &secret)
                .and_then(|_| p.get(&acct))
                .map(|got| got.as_ref() == Some(&secret));
            match stored {
                Ok(true) => {
                    // One answer, not two that can disagree.
                    let _ = self.fallback.delete(&acct);
                    return Ok(self.mode());
                }
                Ok(false) => tracing::warn!("keychain did not keep the {provider:?} key"),
                Err(e) => tracing::warn!("keychain write failed for {provider:?}: {e}"),
            }
        }
        self.fallback.set(&acct, &secret)?;
        Ok(StoreMode {
            backend: self.fallback.kind(),
            fallback: true,
        })
    }

    pub fn clear(&self, provider: ProviderId) -> AppResult<()> {
        let acct = account(provider);
        let mut first_err = None;
        if let Some(p) = self.primary.as_deref() {
            if let Err(e) = p.delete(&acct) {
                tracing::warn!("keychain delete failed for {provider:?}: {e}");
                first_err = Some(e);
            }
        }
        self.fallback.delete(&acct)?;
        match first_err {
            // The file copy is gone; a keychain that is down cannot hold one.
            Some(_) if self.usable_primary().is_none() => Ok(()),
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    pub fn get(&self, provider: ProviderId) -> Option<Secret> {
        let acct = account(provider);
        if let Some(p) = self.usable_primary() {
            match p.get(&acct) {
                Ok(Some(s)) => return Some(s),
                Ok(None) => {}
                Err(e) => tracing::warn!("keychain read failed for {provider:?}: {e}"),
            }
        }
        match self.fallback.get(&acct) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("credential file unreadable for {provider:?}: {e}");
                None
            }
        }
    }
}

impl CredentialLookup for CredentialStore {
    fn user_key(&self, provider: ProviderId) -> Option<Secret> {
        self.get(provider)
    }
}

/// Which credential a provider would be called with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CredentialSource {
    /// The user's own key, held on this machine.
    User,
    /// The TMC site holds the key and proxies the call.
    Site,
    None,
}

/// One provider's line in the settings screen. No secret, ever.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCredential {
    pub provider: ProviderId,
    pub label: String,
    pub configured: bool,
    pub source: CredentialSource,
    /// The user can add a key of their own for this provider.
    pub accepts_user_key: bool,
    /// Why it is not configured, or how it is reached.
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialReport {
    pub backend: BackendKind,
    /// True when the OS keychain is not in use (encrypted-file mode).
    pub fallback: bool,
    pub providers: Vec<ProviderCredential>,
}

/// Build the report. Pure over the store and the site's answer.
pub fn report(store: &CredentialStore, avail: &ProviderAvailability) -> CredentialReport {
    let mode = store.mode();
    let providers = [ProviderId::Curseforge, ProviderId::Nexus, ProviderId::Modio]
        .into_iter()
        .map(|p| {
            let user = accepts_user_key(p) && store.get(p).is_some();
            let site = !accepts_user_key(p) && avail.is_enabled(p);
            let (source, note) = if user {
                (
                    CredentialSource::User,
                    "Using the key stored on this computer.".to_string(),
                )
            } else if site {
                (
                    CredentialSource::Site,
                    "Provided by the TMC site; nothing to set up.".to_string(),
                )
            } else if accepts_user_key(p) {
                (
                    CredentialSource::None,
                    format!("{} needs your own API key.", p.label()),
                )
            } else {
                (
                    CredentialSource::None,
                    avail.reason.clone().unwrap_or_else(|| {
                        format!("{} is not enabled on the TMC site.", p.label())
                    }),
                )
            };
            ProviderCredential {
                provider: p,
                label: p.label().to_string(),
                configured: source != CredentialSource::None,
                source,
                accepts_user_key: accepts_user_key(p),
                note,
            }
        })
        .collect();
    CredentialReport {
        backend: mode.backend,
        fallback: mode.fallback,
        providers,
    }
}

/// Shared handle used by the app.
pub type SharedCredentials = Arc<CredentialStore>;

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(primary: MemoryBackend) -> CredentialStore {
        CredentialStore::new(Some(Box::new(primary)), Box::new(MemoryBackend::default()))
    }

    #[test]
    fn set_get_clear_round_trip_on_the_primary() {
        let s = store_with(MemoryBackend::default());
        let mode = s.set(ProviderId::Nexus, "  abc123  ").unwrap();
        assert_eq!(mode.backend, BackendKind::Memory);
        assert!(!mode.fallback);
        assert_eq!(s.get(ProviderId::Nexus).unwrap().expose(), "abc123");
        s.clear(ProviderId::Nexus).unwrap();
        assert!(s.get(ProviderId::Nexus).is_none());
    }

    #[test]
    fn missing_keychain_uses_the_fallback_and_says_so() {
        let s = store_with(MemoryBackend::unavailable());
        assert!(s.mode().fallback);
        let mode = s.set(ProviderId::Nexus, "k").unwrap();
        assert!(mode.fallback);
        assert_eq!(s.get(ProviderId::Nexus).unwrap().expose(), "k");
    }

    #[test]
    fn rejects_empty_control_and_unsupported_providers() {
        let s = store_with(MemoryBackend::default());
        assert!(s.set(ProviderId::Nexus, "   ").is_err());
        assert!(s.set(ProviderId::Nexus, "a\nb").is_err());
        assert!(s.set(ProviderId::Nexus, &"x".repeat(5000)).is_err());
        assert!(s.set(ProviderId::Curseforge, "k").is_err());
        assert!(s.set(ProviderId::Modrinth, "k").is_err());
    }

    #[test]
    fn secrets_do_not_print() {
        let s = Secret::new("hunter2");
        assert!(!format!("{s:?}").contains("hunter2"));
    }

    #[test]
    fn report_has_no_secret_and_reflects_sources() {
        let s = store_with(MemoryBackend::default());
        let avail = ProviderAvailability::from_json(&serde_json::json!({
            "providers": {"curseforge": true}
        }));
        let before = report(&s, &avail);
        let nexus = |r: &CredentialReport| {
            r.providers
                .iter()
                .find(|p| p.provider == ProviderId::Nexus)
                .unwrap()
                .clone()
        };
        assert!(!nexus(&before).configured);
        let cf = before
            .providers
            .iter()
            .find(|p| p.provider == ProviderId::Curseforge)
            .unwrap();
        assert_eq!(cf.source, CredentialSource::Site);
        s.set(ProviderId::Nexus, "topsecretvalue").unwrap();
        let after = report(&s, &avail);
        assert_eq!(nexus(&after).source, CredentialSource::User);
        let json = serde_json::to_string(&after).unwrap();
        assert!(!json.contains("topsecretvalue"));
        let modio = after
            .providers
            .iter()
            .find(|p| p.provider == ProviderId::Modio)
            .unwrap();
        assert_eq!(modio.source, CredentialSource::None);
    }

    #[test]
    fn a_dead_primary_with_a_stale_file_copy_still_clears() {
        let s = store_with(MemoryBackend::unavailable());
        s.set(ProviderId::Nexus, "k").unwrap();
        s.clear(ProviderId::Nexus).unwrap();
        assert!(s.get(ProviderId::Nexus).is_none());
    }
}
