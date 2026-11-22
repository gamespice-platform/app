//! Steam Workshop: no key, but the download belongs to the Steam client, so
//! resolving hands back the Workshop page in `manual_only`.

use futures_util::future::BoxFuture;

use crate::error::AppResult;
use crate::external::provider::{Fetch, ModProvider, ResolveCtx};
use crate::external::{ExternalRef, GameHint, ProviderId, Resolved};

pub struct Steam;

impl ModProvider for Steam {
    fn id(&self) -> ProviderId {
        ProviderId::Steam
    }

    fn resolve<'a>(
        &'a self,
        _fetch: &'a dyn Fetch,
        r: &'a ExternalRef,
        _ctx: &'a ResolveCtx,
    ) -> BoxFuture<'a, AppResult<Resolved>> {
        Box::pin(async move {
            let item = r.project.clone().unwrap_or_default();
            let page = format!("https://steamcommunity.com/sharedfiles/filedetails/?id={item}");
            Ok(Resolved {
                provider: ProviderId::Steam,
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
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::provider::MockFetch;

    #[tokio::test]
    async fn steam_hands_off_to_the_page() {
        let f = MockFetch::default();
        let r = ExternalRef {
            project: Some("42".into()),
            ..ExternalRef::new(ProviderId::Steam, "u")
        };
        let got = Steam.resolve(&f, &r, &ResolveCtx::default()).await.unwrap();
        assert!(got.manual_only.unwrap().ends_with("?id=42"));
        assert!(got.files.is_empty());
    }
}
