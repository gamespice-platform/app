//! Nexus Mods, with the user's own API key.
//!
//! Nexus keys are per person and tied to their account, so unlike CurseForge
//! and mod.io there is no site-held key: the user's key is read from the
//! credential store and sent as the `apikey` header to `api.nexusmods.com`
//! only.
//!
//! Downloads: a premium account can ask for a link directly. A free account
//! may only download through the site's "Mod Manager Download" button, which
//! opens an `nxm://` link carrying a one-off `key` and `expires`; with those
//! the same endpoint works. A free user who pastes a plain page link gets the
//! page back (`manual_only`) with the reason, instead of an error.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::Value;

use super::{ref_from_installed, s};
use crate::error::{AppError, AppResult};
use crate::external::credentials::CredentialLookup;
use crate::external::provider::{Fetch, ModProvider, ProviderAvailability, ResolveCtx};
use crate::external::{
    CredentialState, ExternalRef, ExternalSource, GameHint, Hashes, ProviderId, RemoteFile,
    Resolved,
};

const API: &str = "https://api.nexusmods.com/v1";

pub struct Nexus {
    creds: Arc<dyn CredentialLookup>,
}

impl Nexus {
    pub fn new(creds: Arc<dyn CredentialLookup>) -> Self {
        Self { creds }
    }
}

/// Game domain: lowercase letters, digits, `_`, `-`.
fn safe_domain(game: &str) -> AppResult<&str> {
    if !game.is_empty()
        && game
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        Ok(game)
    } else {
        Err(AppError::invalid("that is not a Nexus game name"))
    }
}

fn safe_id(id: &str) -> AppResult<&str> {
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
        Ok(id)
    } else {
        Err(AppError::invalid("that is not a Nexus id"))
    }
}

/// `mods/266` or `266` to `266`.
fn mod_id(project: &str) -> &str {
    project.rsplit('/').next().unwrap_or(project)
}

/// The newest main file, else the newest that is not archived or deleted.
pub(crate) fn pick_file(files: &Value) -> Option<&Value> {
    let list = files.get("files")?.as_array()?;
    let newest = |cat: &dyn Fn(&str) -> bool| {
        list.iter()
            .filter(|f| cat(&s(f, "category_name").unwrap_or_default().to_uppercase()))
            .max_by_key(|f| {
                f.get("uploaded_timestamp")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            })
    };
    newest(&|c| c == "MAIN").or_else(|| newest(&|c| !matches!(c, "ARCHIVED" | "OLD_VERSION")))
}

fn file_by_id<'a>(files: &'a Value, id: &str) -> Option<&'a Value> {
    files
        .get("files")?
        .as_array()?
        .iter()
        .find(|f| f.get("file_id").map(|v| v.to_string()).as_deref() == Some(id))
}

fn page(game: &str, id: &str, file: &str) -> String {
    format!("https://www.nexusmods.com/{game}/mods/{id}?tab=files&file_id={file}")
}

/// Map the three Nexus answers to a [`Resolved`]. Pure. `link` is the CDN URL,
/// or `None` when the app may not fetch one (a free account with no nxm key).
pub fn map(
    game: &str,
    id: &str,
    module: &Value,
    file: &Value,
    link: Option<String>,
) -> AppResult<Resolved> {
    let file_id = file
        .get("file_id")
        .map(|v| v.to_string())
        .ok_or_else(|| AppError::invalid("Nexus sent a file with no id"))?;
    let name = s(file, "file_name").ok_or_else(|| AppError::invalid("Nexus sent no file name"))?;
    let size = file
        .get("size_in_bytes")
        .and_then(Value::as_u64)
        .or_else(|| {
            file.get("size_kb")
                .and_then(Value::as_u64)
                .map(|k| k.saturating_mul(1024))
        });
    let page_url = page(game, id, &file_id);
    let files = match &link {
        Some(url) => vec![RemoteFile {
            url: url.clone(),
            name,
            size,
            hashes: Hashes {
                md5: s(file, "md5"),
                ..Hashes::default()
            },
            primary: true,
        }],
        None => vec![],
    };
    Ok(Resolved {
        provider: ProviderId::Nexus,
        project_id: id.to_string(),
        version_id: file_id,
        title: s(module, "name").unwrap_or_else(|| format!("Nexus mod {id}")),
        version: s(file, "version")
            .or_else(|| s(module, "version"))
            .unwrap_or_default(),
        author: s(module, "author"),
        game: GameHint {
            game: Some(game.to_string()),
            ..GameHint::default()
        },
        files,
        deps: vec![],
        page_url: Some(format!("https://www.nexusmods.com/{game}/mods/{id}")),
        manual_only: link.is_none().then_some(page_url),
        licence: None,
    })
}

/// First `URI` in `download_link.json`'s answer, https only.
pub(crate) fn link_from(body: &Value) -> AppResult<String> {
    let url = body
        .as_array()
        .and_then(|a| a.first())
        .and_then(|e| s(e, "URI"))
        .ok_or_else(|| AppError::invalid("Nexus gave no download link"))?;
    if !url.starts_with("https://") {
        return Err(AppError::Network(
            "Nexus sent a link that is not https".into(),
        ));
    }
    Ok(url)
}

impl ModProvider for Nexus {
    fn id(&self) -> ProviderId {
        ProviderId::Nexus
    }

    fn credential_state(&self, _avail: &ProviderAvailability) -> CredentialState {
        if self.creds.user_key(ProviderId::Nexus).is_some() {
            CredentialState::Configured
        } else {
            CredentialState::NotConfigured {
                reason: "Nexus Mods needs your own API key. Add it in Settings.".into(),
            }
        }
    }

    fn resolve<'a>(
        &'a self,
        fetch: &'a dyn Fetch,
        r: &'a ExternalRef,
        _ctx: &'a ResolveCtx,
    ) -> BoxFuture<'a, AppResult<Resolved>> {
        Box::pin(async move {
            let key = self.creds.user_key(ProviderId::Nexus).ok_or_else(|| {
                AppError::invalid("Nexus Mods needs your own API key. Add it in Settings.")
            })?;
            let game = safe_domain(r.game.as_deref().unwrap_or_default())?;
            let id = safe_id(mod_id(r.project.as_deref().unwrap_or_default()))?;
            let headers = [
                ("apikey", key.expose()),
                ("Application-Name", "GameSpiceApp"),
                ("Application-Version", env!("CARGO_PKG_VERSION")),
            ];
            let module = fetch
                .get_json_with(&format!("{API}/games/{game}/mods/{id}.json"), &headers)
                .await?;
            let files = fetch
                .get_json_with(
                    &format!("{API}/games/{game}/mods/{id}/files.json"),
                    &headers,
                )
                .await?;
            let file = match r.version.as_deref() {
                Some(v) => file_by_id(&files, safe_id(v)?)
                    .ok_or_else(|| AppError::invalid("Nexus has no such file for that mod"))?,
                None => pick_file(&files)
                    .ok_or_else(|| AppError::invalid("that Nexus mod has no downloadable file"))?,
            };
            let file_id = file
                .get("file_id")
                .map(|v| v.to_string())
                .unwrap_or_default();
            let file_id = safe_id(&file_id)?;
            let link_url =
                format!("{API}/games/{game}/mods/{id}/files/{file_id}/download_link.json");
            // nxm key + expires: what a free account is given by the site's
            // "Download with manager" button. Both or neither.
            let nxm = match (r.extras.get("key"), r.extras.get("expires")) {
                (Some(k), Some(e)) => Some((k, e)),
                _ => None,
            };
            let link = if let Some((k, e)) = nxm {
                let query = {
                    let mut q = url::form_urlencoded::Serializer::new(String::new());
                    q.append_pair("key", k).append_pair("expires", e);
                    if let Some(u) = r.extras.get("user_id") {
                        q.append_pair("user_id", u);
                    }
                    q.finish()
                };
                Some(link_from(
                    &fetch
                        .get_json_with(&format!("{link_url}?{query}"), &headers)
                        .await?,
                )?)
            } else {
                let me = fetch
                    .get_json_with(&format!("{API}/users/validate.json"), &headers)
                    .await?;
                if me
                    .get("is_premium")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    Some(link_from(&fetch.get_json_with(&link_url, &headers).await?)?)
                } else {
                    None
                }
            };
            map(game, id, &module, file, link)
        })
    }

    fn latest<'a>(
        &'a self,
        fetch: &'a dyn Fetch,
        installed: &'a ExternalSource,
        ctx: &'a ResolveCtx,
    ) -> BoxFuture<'a, AppResult<Option<Resolved>>> {
        Box::pin(async move {
            let mut r = ref_from_installed(installed);
            // The installed link may have been an nxm file link; ask for the
            // newest main file, and never reuse a spent key.
            r.version = None;
            r.extras.clear();
            let latest = self.resolve(fetch, &r, ctx).await?;
            Ok((latest.version_id != installed.version_id).then_some(latest))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::credentials::{CredentialStore, MemoryBackend};
    use crate::external::provider::MockFetch;
    use serde_json::json;

    fn with_key() -> Nexus {
        let store = CredentialStore::new(
            Some(Box::new(MemoryBackend::default())),
            Box::new(MemoryBackend::default()),
        );
        store.set(ProviderId::Nexus, "KEY123").unwrap();
        Nexus::new(Arc::new(store))
    }

    fn fetch(premium: bool) -> MockFetch {
        let mut f = MockFetch::default();
        let a = &mut f.answers;
        a.insert(
            format!("{API}/games/skyrim/mods/266.json"),
            json!({"name": "SkyUI", "version": "5.2", "author": "schlangster"}),
        );
        a.insert(
            format!("{API}/games/skyrim/mods/266/files.json"),
            json!({"files": [
                {"file_id": 1, "file_name": "old.zip", "category_name": "OLD_VERSION",
                 "uploaded_timestamp": 9, "version": "4"},
                {"file_id": 2, "file_name": "SkyUI.zip", "category_name": "MAIN",
                 "uploaded_timestamp": 5, "version": "5.2", "size_in_bytes": 100,
                 "md5": "abc"},
                {"file_id": 3, "file_name": "older.zip", "category_name": "MAIN",
                 "uploaded_timestamp": 1, "version": "5.0"}
            ]}),
        );
        a.insert(
            format!("{API}/users/validate.json"),
            json!({"is_premium": premium}),
        );
        a.insert(
            format!("{API}/games/skyrim/mods/266/files/2/download_link.json"),
            json!([{"name": "CF", "URI": "https://cdn.nexus/a.zip"}]),
        );
        a.insert(
            format!(
                "{API}/games/skyrim/mods/266/files/2/download_link.json?key=K&expires=99&user_id=7"
            ),
            json!([{"name": "CF", "URI": "https://cdn.nexus/free.zip"}]),
        );
        f
    }

    fn r() -> ExternalRef {
        let mut r = ExternalRef::new(
            ProviderId::Nexus,
            "https://www.nexusmods.com/skyrim/mods/266",
        );
        r.game = Some("skyrim".into());
        r.project = Some("266".into());
        r
    }

    #[tokio::test]
    async fn premium_gets_a_link_and_the_key_goes_only_in_the_header() {
        let f = fetch(true);
        let got = with_key()
            .resolve(&f, &r(), &ResolveCtx::default())
            .await
            .unwrap();
        assert_eq!(got.version_id, "2");
        assert_eq!(got.files[0].url, "https://cdn.nexus/a.zip");
        assert_eq!(got.files[0].hashes.md5.as_deref(), Some("abc"));
        assert!(got.manual_only.is_none());
        let seen = f.seen.lock().unwrap();
        for (url, headers) in seen.iter() {
            assert!(url.starts_with("https://api.nexusmods.com/"));
            assert!(!url.contains("KEY123"));
            assert!(headers.iter().any(|(k, v)| k == "apikey" && v == "KEY123"));
        }
    }

    #[tokio::test]
    async fn free_without_an_nxm_key_gets_the_page() {
        let f = fetch(false);
        let got = with_key()
            .resolve(&f, &r(), &ResolveCtx::default())
            .await
            .unwrap();
        assert!(got.files.is_empty());
        assert!(got.manual_only.unwrap().contains("file_id=2"));
    }

    #[tokio::test]
    async fn free_with_an_nxm_key_downloads() {
        let f = fetch(false);
        let mut r = r();
        r.version = Some("2".into());
        r.extras.insert("key".into(), "K".into());
        r.extras.insert("expires".into(), "99".into());
        r.extras.insert("user_id".into(), "7".into());
        let got = with_key()
            .resolve(&f, &r, &ResolveCtx::default())
            .await
            .unwrap();
        assert_eq!(got.files[0].url, "https://cdn.nexus/free.zip");
        // No premium probe was needed.
        assert!(!f
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|(u, _)| u.contains("validate")));
    }

    #[tokio::test]
    async fn no_key_means_not_configured_and_no_request() {
        let n = Nexus::new(Arc::new(crate::external::credentials::NoCredentials));
        assert!(!n
            .credential_state(&ProviderAvailability::disabled())
            .usable());
        let f = fetch(true);
        assert!(n.resolve(&f, &r(), &ResolveCtx::default()).await.is_err());
        assert!(f.seen.lock().unwrap().is_empty());
        assert!(with_key()
            .credential_state(&ProviderAvailability::disabled())
            .usable());
    }

    #[tokio::test]
    async fn hostile_game_and_id_are_refused() {
        let f = fetch(true);
        let mut bad = r();
        bad.game = Some("../x".into());
        assert!(with_key()
            .resolve(&f, &bad, &ResolveCtx::default())
            .await
            .is_err());
        let mut bad = r();
        bad.project = Some("1/../2".into());
        assert!(with_key()
            .resolve(&f, &bad, &ResolveCtx::default())
            .await
            .is_err());
    }

    #[test]
    fn insecure_links_are_refused() {
        assert!(link_from(&json!([{"URI": "http://x/a"}])).is_err());
        assert!(link_from(&json!([])).is_err());
    }
}
