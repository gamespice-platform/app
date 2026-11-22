//! CurseForge and mod.io. The API keys are the TMC site's, so the app asks the
//! site ([`crate::external::site`]) and downloads from the CDN URL it returns.
//! No key is held, read or sent from here.

use std::sync::Arc;

use futures_util::future::BoxFuture;

use super::ref_from_installed;
use crate::error::AppResult;
use crate::external::provider::{Fetch, ModProvider, ProviderAvailability, ResolveCtx};
use crate::external::site::{self, SiteApi};
use crate::external::{CredentialState, ExternalRef, ExternalSource, ProviderId, Resolved};

pub struct Proxied {
    id: ProviderId,
    site: Arc<dyn SiteApi>,
}

impl Proxied {
    pub fn new(id: ProviderId, site: Arc<dyn SiteApi>) -> Self {
        debug_assert!(matches!(id, ProviderId::Curseforge | ProviderId::Modio));
        Self { id, site }
    }
}

impl ModProvider for Proxied {
    fn id(&self) -> ProviderId {
        self.id
    }

    fn credential_state(&self, avail: &ProviderAvailability) -> CredentialState {
        if avail.is_enabled(self.id) {
            return CredentialState::Configured;
        }
        let reason = match (&avail.reason, avail.from_site) {
            (Some(why), _) => why.clone(),
            (None, true) => format!("{} is turned off on the TMC site.", self.id.label()),
            (None, false) => format!(
                "{} is not enabled yet: the TMC site has not said it can broker it.",
                self.id.label()
            ),
        };
        CredentialState::NotConfigured { reason }
    }

    fn resolve<'a>(
        &'a self,
        _fetch: &'a dyn Fetch,
        r: &'a ExternalRef,
        _ctx: &'a ResolveCtx,
    ) -> BoxFuture<'a, AppResult<Resolved>> {
        Box::pin(async move { site::resolve(self.site.as_ref(), r).await })
    }

    fn latest<'a>(
        &'a self,
        fetch: &'a dyn Fetch,
        installed: &'a ExternalSource,
        ctx: &'a ResolveCtx,
    ) -> BoxFuture<'a, AppResult<Option<Resolved>>> {
        Box::pin(async move {
            let r = ref_from_installed(installed);
            let latest = self.resolve(fetch, &r, ctx).await?;
            Ok((latest.version_id != installed.version_id).then_some(latest))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::provider::MockFetch;
    use crate::external::site::{resolve_path, MockSite, NoSite};
    use serde_json::json;

    fn answer(v: &str) -> serde_json::Value {
        json!({
            "provider": "curseforge", "projectId": "9", "versionId": v,
            "title": "T", "version": v, "author": null,
            "game": {"game": "minecraft", "loaders": [], "gameVersions": []},
            "files": [{"url": "https://edge.forgecdn.net/a.jar", "name": "a.jar",
                       "size": 1, "hashes": {"sha1": "x"}, "primary": true}],
            "deps": [], "pageUrl": null, "manualOnly": null, "licence": null
        })
    }

    #[tokio::test]
    async fn resolves_through_the_site_with_no_key_in_the_request() {
        let mut r = ExternalRef::new(ProviderId::Curseforge, "https://www.curseforge.com/x");
        r.project = Some("9".into());
        let mut site = MockSite::default();
        site.answers.insert(resolve_path(&r), answer("5"));
        let site = Arc::new(site);
        let p = Proxied::new(ProviderId::Curseforge, site.clone());
        let got = p
            .resolve(&MockFetch::default(), &r, &ResolveCtx::default())
            .await
            .unwrap();
        assert_eq!(got.version_id, "5");
        let seen = site.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(!seen[0].to_lowercase().contains("key"));
    }

    #[tokio::test]
    async fn latest_is_none_when_current() {
        let mut site = MockSite::default();
        let installed = ExternalSource {
            provider: ProviderId::Curseforge,
            url: "https://www.curseforge.com/minecraft/mc-mods/foo".into(),
            project_id: "9".into(),
            version_id: "5".into(),
            version: "5".into(),
        };
        let r = ref_from_installed(&installed);
        site.answers.insert(resolve_path(&r), answer("5"));
        let p = Proxied::new(ProviderId::Curseforge, Arc::new(site));
        let got = p
            .latest(&MockFetch::default(), &installed, &ResolveCtx::default())
            .await
            .unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn disabled_says_why() {
        let p = Proxied::new(ProviderId::Modio, Arc::new(NoSite));
        let st = p.credential_state(&ProviderAvailability::unreachable("offline"));
        assert!(matches!(st, CredentialState::NotConfigured { reason } if reason == "offline"));
        let st = p.credential_state(&ProviderAvailability::disabled());
        assert!(!st.usable());
        let on = ProviderAvailability::from_json(&json!({"providers": {"modio": true}}));
        assert!(p.credential_state(&on).usable());
    }
}
