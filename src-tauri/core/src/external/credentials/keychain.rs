//! The OS credential store as a [`SecretBackend`].

use super::{BackendKind, Secret, SecretBackend};
use crate::error::{AppError, AppResult};

const SERVICE: &str = crate::rename::KEYRING_SERVICE;
/// An account that never holds anything; reading it tells us whether a
/// credential service answers at all.
const PROBE: &str = "external-probe";

pub struct Keychain;

impl Keychain {
    pub fn new() -> Self {
        Self
    }

    /// Same split as the sign-in token: a dev build's keys are not the real
    /// build's.
    fn scoped(account: &str) -> String {
        match crate::api::api_base_scope() {
            Some(scope) => format!("{account}@{scope}"),
            None => account.to_string(),
        }
    }

    fn entry(account: &str) -> AppResult<keyring::Entry> {
        keyring::Entry::new(SERVICE, &Self::scoped(account))
            .map_err(|e| AppError::internal(format!("credential store: {e}")))
    }
}

impl Default for Keychain {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretBackend for Keychain {
    fn kind(&self) -> BackendKind {
        BackendKind::Keychain
    }

    fn available(&self) -> bool {
        match Self::entry(PROBE).and_then(|e| {
            e.get_password().map(|_| ()).or_else(|err| match err {
                keyring::Error::NoEntry => Ok(()),
                other => Err(AppError::internal(format!("credential store: {other}"))),
            })
        }) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("no OS credential store: {e}");
                false
            }
        }
    }

    fn get(&self, account: &str) -> AppResult<Option<Secret>> {
        match Self::entry(account)?.get_password() {
            Ok(v) => Ok(Some(Secret::new(v))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AppError::internal(format!("credential store read: {e}"))),
        }
    }

    fn set(&self, account: &str, value: &Secret) -> AppResult<()> {
        Self::entry(account)?
            .set_password(value.expose())
            .map_err(|e| AppError::internal(format!("credential store write: {e}")))
    }

    fn delete(&self, account: &str) -> AppResult<()> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AppError::internal(format!("credential store delete: {e}"))),
        }
    }
}
