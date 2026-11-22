//! The provider trait, the registry, and what decides whether a keyed provider is on.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::providers;
use super::{CredentialState, ExternalRef, ExternalSource, ProviderId, Resolved};
use crate::error::{AppError, AppResult};

/// A JSON GET. The only network surface the providers use, so tests hand in a
/// [`MockFetch`] and nothing in this module's tests touches a socket.
pub trait Fetch: Send + Sync {
    /// A GET carrying extra request headers (a provider's API key). Only the
    /// providers that hold the user's own credential use this; the headers go
    /// to the URL's host and nowhere else.
    fn get_json_with<'a>(
        &'a self,
        url: &'a str,
        headers: &'a [(&'a str, &'a str)],
    ) -> BoxFuture<'a, AppResult<Value>>;

    fn get_json<'a>(&'a self, url: &'a str) -> BoxFuture<'a, AppResult<Value>> {
        self.get_json_with(url, &[])
    }
}

/// The real thing: `reqwest` with the identifying `User-Agent`.
pub struct HttpFetch {
    client: reqwest::Client,
}

impl HttpFetch {
    pub fn new() -> Self {
        let client = crate::http::builder()
            .user_agent(super::user_agent())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| crate::http::client());
        Self { client }
    }
}

impl Default for HttpFetch {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetch for HttpFetch {
    fn get_json_with<'a>(
        &'a self,
        url: &'a str,
        headers: &'a [(&'a str, &'a str)],
    ) -> BoxFuture<'a, AppResult<Value>> {
        Box::pin(async move {
            if !url.starts_with("https://") {
                return Err(AppError::invalid("provider lookups must use https"));
            }
            let mut req = self.client.get(url).header("accept", "application/json");
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| AppError::Network(format!("could not reach {url}: {e}")))?;
            let status = resp.status();
            if status.as_u16() == 404 {
                return Err(AppError::invalid("the provider has no such mod or version"));
            }
            if status.as_u16() == 401 {
                return Err(AppError::invalid(format!(
                    "{} rejected the credential ({status}); check the key in Settings",
                    url.split('/').nth(2).unwrap_or("the provider")
                )));
            }
            if status.as_u16() == 429 || status.as_u16() == 403 {
                return Err(AppError::Network(format!(
                    "{} refused the request ({status}); it is rate limiting this address, try again later",
                    url.split('/').nth(2).unwrap_or("the provider")
                )));
            }
            if !status.is_success() {
                return Err(AppError::Network(format!("the provider answered {status}")));
            }
            resp.json::<Value>().await.map_err(|e| {
                AppError::Network(format!("the provider sent something unreadable: {e}"))
            })
        })
    }
}

/// One request a [`MockFetch`] saw: the URL and the headers sent.
pub type SeenRequest = (String, Vec<(String, String)>);

/// Canned answers by exact URL, for tests.
#[derive(Default)]
pub struct MockFetch {
    pub answers: BTreeMap<String, Value>,
    /// Every `(url, headers)` asked for, so a test can see what was sent.
    pub seen: std::sync::Mutex<Vec<SeenRequest>>,
}

impl Fetch for MockFetch {
    fn get_json_with<'a>(
        &'a self,
        url: &'a str,
        headers: &'a [(&'a str, &'a str)],
    ) -> BoxFuture<'a, AppResult<Value>> {
        Box::pin(async move {
            if let Ok(mut seen) = self.seen.lock() {
                seen.push((
                    url.to_string(),
                    headers
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                ));
            }
            self.answers
                .get(url)
                .cloned()
                .ok_or_else(|| AppError::invalid(format!("mock: no answer for {url}")))
        })
    }
}

/// What the caller wants the version to suit. All optional.
#[derive(Debug, Clone, Default)]
pub struct ResolveCtx {
    pub loader: Option<String>,
    pub game_version: Option<String>,
}

/// Which keyed providers the TMC AdminCP has switched on.
///
/// Fetched from `GET /api/app/v1/external/providers` (contract in
/// `docs/external-mods.md`; see [`super::site`]). Until the site answers every
/// proxied provider is "not enabled". No key is held here: the flag says the
/// *server* can broker the call, not that the app has a key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAvailability {
    pub enabled: BTreeMap<ProviderId, bool>,
    /// False when the site did not answer (no endpoint yet, offline, signed
    /// out), so the UI can say "not enabled yet" rather than "disabled by the
    /// admin".
    pub from_site: bool,
    /// Why the site could not say, when `from_site` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ProviderAvailability {
    /// Nothing proxied is on.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Nothing is on because the site could not be asked.
    pub fn unreachable(reason: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
            ..Self::default()
        }
    }

    pub fn from_json(v: &Value) -> Self {
        let mut enabled = BTreeMap::new();
        if let Some(map) = v.get("providers").and_then(Value::as_object) {
            for (k, on) in map {
                if let (Some(id), Some(on)) = (ProviderId::parse(k), on.as_bool()) {
                    enabled.insert(id, on);
                }
            }
        }
        Self {
            enabled,
            from_site: true,
            reason: None,
        }
    }

    pub fn is_enabled(&self, id: ProviderId) -> bool {
        self.enabled.get(&id).copied().unwrap_or(false)
    }
}

/// One source of mods.
pub trait ModProvider: Send + Sync {
    fn id(&self) -> ProviderId;

    /// Does this provider own the pasted text? Pure.
    fn parse(&self, text: &str) -> Option<ExternalRef> {
        super::parse::parse_input(text)
            .ok()
            .filter(|r| r.provider == self.id())
    }

    /// Look the reference up.
    fn resolve<'a>(
        &'a self,
        fetch: &'a dyn Fetch,
        r: &'a ExternalRef,
        ctx: &'a ResolveCtx,
    ) -> BoxFuture<'a, AppResult<Resolved>>;

    /// The newest version of what is installed, or `None` when it is current.
    fn latest<'a>(
        &'a self,
        fetch: &'a dyn Fetch,
        installed: &'a ExternalSource,
        ctx: &'a ResolveCtx,
    ) -> BoxFuture<'a, AppResult<Option<Resolved>>> {
        Box::pin(async move {
            let mut r = ExternalRef::new(self.id(), installed.url.clone());
            r.project = Some(installed.project_id.clone());
            let latest = self.resolve(fetch, &r, ctx).await?;
            Ok((latest.version_id != installed.version_id).then_some(latest))
        })
    }

    fn credential_state(&self, avail: &ProviderAvailability) -> CredentialState {
        let _ = avail;
        CredentialState::NotRequired
    }
}

/// The providers, by id.
pub struct Registry {
    providers: Vec<Arc<dyn ModProvider>>,
}

impl Registry {
    /// No site and no user credentials: the keyed providers report that
    /// plainly. For parsing and the providers that need neither.
    pub fn standard() -> Self {
        Self::with_services(&super::Services::none())
    }

    pub fn with_services(services: &super::Services) -> Self {
        Self {
            providers: providers::standard(services),
        }
    }

    pub fn get(&self, id: ProviderId) -> Option<&Arc<dyn ModProvider>> {
        self.providers.iter().find(|p| p.id() == id)
    }

    pub fn all(&self) -> &[Arc<dyn ModProvider>] {
        &self.providers
    }

    /// Credential state per provider, for the settings screen.
    pub fn states(&self, avail: &ProviderAvailability) -> Vec<(ProviderId, CredentialState)> {
        self.providers
            .iter()
            .map(|p| (p.id(), p.credential_state(avail)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_availability_disables_keyed_providers() {
        let reg = Registry::standard();
        let a = ProviderAvailability::disabled();
        for (id, st) in reg.states(&a) {
            match id {
                ProviderId::Curseforge | ProviderId::Nexus | ProviderId::Modio => {
                    assert!(
                        matches!(st, CredentialState::NotConfigured { .. }),
                        "{id:?}"
                    );
                }
                _ => assert!(st.usable(), "{id:?}"),
            }
        }
    }

    #[test]
    fn availability_json_turns_a_provider_on() {
        let a = ProviderAvailability::from_json(&serde_json::json!({
            "providers": {"curseforge": true, "nexus": false, "bogus": true}
        }));
        assert!(a.from_site && a.is_enabled(ProviderId::Curseforge));
        assert!(!a.is_enabled(ProviderId::Nexus));
        let reg = Registry::standard();
        let st = reg
            .get(ProviderId::Curseforge)
            .unwrap()
            .credential_state(&a);
        assert_eq!(st, CredentialState::Configured);
    }
}
