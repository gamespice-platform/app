//! Providers that need a key (CurseForge, Nexus, mod.io) or a client (Steam).
//!
//! The three keyed ones are unusable until the TMC AdminCP switches them on
//! ([`ProviderAvailability`]); even then lookups are not wired in this build,
//! and say so rather than pretending. No key is stored or read here.
//! Steam is the other shape: no key, but the download belongs to the Steam
//! client, so resolving hands back the Workshop page in `manual_only`.

use futures_util::future::BoxFuture;

use crate::error::{AppError, AppResult};
use crate::external::provider::{Fetch, ModProvider, ProviderAvailability, ResolveCtx};
use crate::external::{CredentialState, ExternalRef, GameHint, ProviderId, Resolved};

pub struct Keyed {
    id: ProviderId,
}

impl Keyed {
    pub fn curseforge() -> Self {
        Self {
            id: ProviderId::Curseforge,
        }
    }
    pub fn nexus() -> Self {
        Self {
            id: ProviderId::Nexus,
        }
    }
    pub fn modio() -> Self {
        Self {
            id: ProviderId::Modio,
        }
    }
    pub fn steam() -> Self {
        Self {
            id: ProviderId::Steam,
        }
    }
}

impl ModProvider for Keyed {
    fn id(&self) -> ProviderId {
        self.id
    }

    fn credential_state(&self, avail: &ProviderAvailability) -> CredentialState {
        if self.id == ProviderId::Steam {
            return CredentialState::NotRequired;
        }
        if avail.is_enabled(self.id) {
            CredentialState::Configured
        } else {
            CredentialState::NotConfigured {
                reason: format!(
                    "{} needs an API key that the Modding Community admin has not enabled yet",
                    self.id.label()
                ),
            }
        }
    }

    fn resolve<'a>(
        &'a self,
        _fetch: &'a dyn Fetch,
        r: &'a ExternalRef,
        _ctx: &'a ResolveCtx,
    ) -> BoxFuture<'a, AppResult<Resolved>> {
        let id = self.id;
        Box::pin(async move {
            if id == ProviderId::Steam {
                let item = r.project.clone().unwrap_or_default();
                let page = format!("https://steamcommunity.com/sharedfiles/filedetails/?id={item}");
                return Ok(Resolved {
                    provider: id,
                    project_id: item.clone(),
                    version_id: item,
                    title: "Steam Workshop item".into(),
                    version: String::new(),
                    author: None,
                    game: GameHint::default(),
                    files: vec![],
                    deps: vec![],
                    page_url: Some(page.clone()),
                    manual_only: Some(page),
                    licence: None,
                });
            }
            Err(AppError::invalid(format!(
                "{} links cannot be installed yet: they need an API key that is not enabled",
                id.label()
            )))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::provider::MockFetch;

    #[tokio::test]
    async fn keyed_refuses_and_steam_hands_off() {
        let f = MockFetch::default();
        let mut r = ExternalRef::new(ProviderId::Nexus, "u");
        assert!(Keyed::nexus()
            .resolve(&f, &r, &ResolveCtx::default())
            .await
            .is_err());
        r = ExternalRef {
            project: Some("42".into()),
            ..ExternalRef::new(ProviderId::Steam, "u")
        };
        let got = Keyed::steam()
            .resolve(&f, &r, &ResolveCtx::default())
            .await
            .unwrap();
        assert!(got.manual_only.unwrap().ends_with("?id=42"));
        assert!(got.files.is_empty());
    }

    #[test]
    fn nexus_not_configured_until_enabled() {
        let k = Keyed::nexus();
        assert!(!k
            .credential_state(&ProviderAvailability::disabled())
            .usable());
    }
}
