//! The TMC site as a broker for the providers whose API keys are ours
//! (CurseForge, mod.io).
//!
//! The keys live in the TMC AdminCP and never reach the app. The app asks the
//! site, signed in with the user's TMC token, for two things:
//!
//! * which providers are switched on ([`fetch_availability`]), and
//! * the metadata and download URL for one link ([`resolve`]).
//!
//! The file itself is downloaded straight from the provider's CDN. The wire
//! contract is in `docs/external-mods.md`; this module is the client half and
//! talks through [`SiteApi`] so tests need no network.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use reqwest::Method;
use serde_json::Value;

use super::provider::ProviderAvailability;
use super::{ExternalRef, ProviderId, Resolved};
use crate::api::ApiClient;
use crate::error::{AppError, AppResult};

/// Path of the availability endpoint, relative to `/api/app/v1`.
pub const PROVIDERS_PATH: &str = "/external/providers";
/// Path of the resolve endpoint, relative to `/api/app/v1`.
pub const RESOLVE_PATH: &str = "/external/resolve";

/// An authenticated GET against the TMC API. Returns the envelope's `data`.
pub trait SiteApi: Send + Sync {
    fn get<'a>(&'a self, path: &'a str) -> BoxFuture<'a, AppResult<Value>>;
}

impl SiteApi for ApiClient {
    fn get<'a>(&'a self, path: &'a str) -> BoxFuture<'a, AppResult<Value>> {
        Box::pin(async move { self.request(Method::GET, path, None, true).await })
    }
}

/// No site: every call fails, so everything proxied reports "not enabled".
pub struct NoSite;

impl SiteApi for NoSite {
    fn get<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, AppResult<Value>> {
        Box::pin(async { Err(AppError::invalid("the TMC site is not available")) })
    }
}

/// Ask the site which proxied providers are on. Never errors: a site that
/// cannot answer means "not enabled", with the reason kept for the UI.
pub async fn fetch_availability(site: &dyn SiteApi) -> ProviderAvailability {
    match site.get(PROVIDERS_PATH).await {
        Ok(v) => ProviderAvailability::from_json(&v),
        Err(AppError::NotAuthenticated) => ProviderAvailability::unreachable(
            "Sign in to your TMC account to use CurseForge and mod.io links.",
        ),
        Err(e) => ProviderAvailability::unreachable(format!(
            "The TMC site could not say which providers are enabled: {e}"
        )),
    }
}

/// Remembers the site's answer so a paste does not cost a round trip each
/// time, and a site that is down is not hammered.
pub struct AvailabilityCache {
    ok_ttl: Duration,
    fail_ttl: Duration,
    slot: tokio::sync::Mutex<Option<(Instant, ProviderAvailability)>>,
}

impl AvailabilityCache {
    pub fn new(ok_ttl: Duration, fail_ttl: Duration) -> Self {
        Self {
            ok_ttl,
            fail_ttl,
            slot: tokio::sync::Mutex::new(None),
        }
    }

    /// Five minutes when the site answered, thirty seconds when it did not.
    pub fn standard() -> Self {
        Self::new(Duration::from_secs(300), Duration::from_secs(30))
    }

    pub async fn get(&self, site: &dyn SiteApi) -> ProviderAvailability {
        let mut slot = self.slot.lock().await;
        if let Some((at, a)) = slot.as_ref() {
            let ttl = if a.from_site {
                self.ok_ttl
            } else {
                self.fail_ttl
            };
            if at.elapsed() < ttl {
                return a.clone();
            }
        }
        let fresh = fetch_availability(site).await;
        *slot = Some((Instant::now(), fresh.clone()));
        fresh
    }

    /// Forget the answer (sign-in changed, user pressed refresh).
    pub async fn invalidate(&self) {
        *self.slot.lock().await = None;
    }
}

/// `/external/resolve?...` for a link.
pub fn resolve_path(r: &ExternalRef) -> String {
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    q.append_pair("provider", r.provider.as_str());
    if let Some(p) = &r.project {
        q.append_pair("project", p);
    }
    if let Some(v) = &r.version {
        q.append_pair("version", v);
    }
    if let Some(g) = &r.game {
        q.append_pair("game", g);
    }
    if let Some(c) = &r.class {
        q.append_pair("class", c);
    }
    format!("{RESOLVE_PATH}?{}", q.finish())
}

/// Turn the site's answer into a [`Resolved`], refusing anything that does not
/// look right. The site is ours, but its answer still decides what gets
/// downloaded, so it is checked like any provider's.
pub fn parse_resolved(provider: ProviderId, body: &Value) -> AppResult<Resolved> {
    let resolved: Resolved = serde_json::from_value(body.clone())
        .map_err(|e| AppError::Network(format!("the TMC site sent an unreadable answer: {e}")))?;
    if resolved.provider != provider {
        return Err(AppError::Network(
            "the TMC site answered for a different provider".into(),
        ));
    }
    for f in &resolved.files {
        if !f.url.starts_with("https://") {
            return Err(AppError::Network(
                "the TMC site sent a download link that is not https".into(),
            ));
        }
    }
    if resolved.manual_only.is_none() && resolved.files.is_empty() {
        return Err(AppError::invalid("that version has no file to download"));
    }
    Ok(resolved)
}

/// Resolve one link through the site.
pub async fn resolve(site: &dyn SiteApi, r: &ExternalRef) -> AppResult<Resolved> {
    let body = site.get(&resolve_path(r)).await?;
    parse_resolved(r.provider, &body)
}

/// A canned site, for tests.
#[derive(Default)]
pub struct MockSite {
    pub answers: std::collections::BTreeMap<String, Value>,
    pub seen: Mutex<Vec<String>>,
    pub signed_out: bool,
}

impl SiteApi for MockSite {
    fn get<'a>(&'a self, path: &'a str) -> BoxFuture<'a, AppResult<Value>> {
        Box::pin(async move {
            if let Ok(mut s) = self.seen.lock() {
                s.push(path.to_string());
            }
            if self.signed_out {
                return Err(AppError::NotAuthenticated);
            }
            self.answers
                .get(path)
                .cloned()
                .ok_or_else(|| AppError::Api("mock: no such endpoint".into()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn site_with(path: &str, v: Value) -> MockSite {
        let mut m = MockSite::default();
        m.answers.insert(path.to_string(), v);
        m
    }

    #[tokio::test]
    async fn availability_reads_the_site_and_defaults_to_off() {
        let on = site_with(PROVIDERS_PATH, json!({"providers": {"curseforge": true}}));
        let a = fetch_availability(&on).await;
        assert!(a.from_site && a.is_enabled(ProviderId::Curseforge));
        assert!(!a.is_enabled(ProviderId::Modio));

        let a = fetch_availability(&NoSite).await;
        assert!(!a.from_site && !a.is_enabled(ProviderId::Curseforge));
        assert!(a.reason.is_some());

        let out = MockSite {
            signed_out: true,
            ..MockSite::default()
        };
        let a = fetch_availability(&out).await;
        assert!(a.reason.unwrap().contains("Sign in"));
    }

    #[tokio::test]
    async fn the_cache_asks_once_until_invalidated() {
        let site = site_with(PROVIDERS_PATH, json!({"providers": {"modio": true}}));
        let cache = AvailabilityCache::new(Duration::from_secs(60), Duration::from_secs(60));
        assert!(cache.get(&site).await.is_enabled(ProviderId::Modio));
        assert!(cache.get(&site).await.is_enabled(ProviderId::Modio));
        assert_eq!(site.seen.lock().unwrap().len(), 1);
        cache.invalidate().await;
        cache.get(&site).await;
        assert_eq!(site.seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn resolve_path_encodes_every_field() {
        let mut r = ExternalRef::new(ProviderId::Curseforge, "u");
        r.project = Some("a b&c".into());
        r.game = Some("minecraft".into());
        r.version = Some("12".into());
        assert_eq!(
            resolve_path(&r),
            "/external/resolve?provider=curseforge&project=a+b%26c&version=12&game=minecraft"
        );
    }

    fn answer(provider: &str, url: &str) -> Value {
        json!({
            "provider": provider, "projectId": "1", "versionId": "2",
            "title": "T", "version": "1.0", "author": null,
            "game": {"game": "minecraft", "loaders": [], "gameVersions": []},
            "files": [{"url": url, "name": "a.jar", "size": 10,
                       "hashes": {"sha1": "abc"}, "primary": true}],
            "deps": [], "pageUrl": null, "manualOnly": null, "licence": null
        })
    }

    #[test]
    fn site_answers_are_checked() {
        let ok = parse_resolved(ProviderId::Curseforge, &answer("curseforge", "https://e/a"));
        assert_eq!(ok.unwrap().files[0].hashes.sha1.as_deref(), Some("abc"));
        assert!(parse_resolved(ProviderId::Modio, &answer("curseforge", "https://e/a")).is_err());
        assert!(
            parse_resolved(ProviderId::Curseforge, &answer("curseforge", "http://e/a")).is_err()
        );
        assert!(parse_resolved(ProviderId::Curseforge, &json!({"x": 1})).is_err());
    }

    #[test]
    fn a_manual_only_answer_needs_no_file() {
        let mut a = answer("curseforge", "https://e/a");
        a["files"] = json!([]);
        assert!(parse_resolved(ProviderId::Curseforge, &a).is_err());
        a["manualOnly"] = json!("https://www.curseforge.com/x");
        assert!(parse_resolved(ProviderId::Curseforge, &a).is_ok());
    }
}
