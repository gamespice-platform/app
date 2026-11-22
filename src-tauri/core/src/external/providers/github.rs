//! GitHub releases. No key, but unauthenticated calls are limited to 60 an
//! hour per address; a refusal is reported as such by `HttpFetch`.

use futures_util::future::BoxFuture;
use serde_json::Value;

use super::s;
use crate::error::{AppError, AppResult};
use crate::external::parse::DIRECT_EXTENSIONS;
use crate::external::provider::{Fetch, ModProvider, ResolveCtx};
use crate::external::{ExternalRef, GameHint, Hashes, ProviderId, RemoteFile, Resolved};

pub struct Github;

pub(crate) fn release_url(repo: &str, tag: Option<&str>) -> String {
    match tag {
        Some(t) => format!("https://api.github.com/repos/{repo}/releases/tags/{t}"),
        None => format!("https://api.github.com/repos/{repo}/releases/latest"),
    }
}

fn installable(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    DIRECT_EXTENSIONS
        .iter()
        .any(|e| n.ends_with(&format!(".{e}")))
}

/// Map a release. `want` names one asset when the link was an asset link.
pub fn map(repo: &str, rel: &Value, want: Option<&str>) -> AppResult<Resolved> {
    let tag =
        s(rel, "tag_name").ok_or_else(|| AppError::invalid("GitHub sent a release with no tag"))?;
    let mut files: Vec<RemoteFile> = rel
        .get("assets")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    let name = s(f, "name")?;
                    if want.is_some_and(|w| w != name) || (want.is_none() && !installable(&name)) {
                        return None;
                    }
                    // `digest` is "sha256:<hex>" where GitHub provides one.
                    let sha256 =
                        s(f, "digest").and_then(|d| d.strip_prefix("sha256:").map(str::to_string));
                    Some(RemoteFile {
                        url: s(f, "browser_download_url")?,
                        name,
                        size: f.get("size").and_then(Value::as_u64),
                        hashes: Hashes {
                            sha256,
                            ..Hashes::default()
                        },
                        primary: false,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if files.is_empty() {
        return Err(AppError::invalid(format!(
            "release {tag} of {repo} has no .zip, .jar or similar file to install"
        )));
    }
    files[0].primary = true;
    Ok(Resolved {
        provider: ProviderId::Github,
        project_id: repo.to_string(),
        version_id: tag.clone(),
        title: repo.rsplit('/').next().unwrap_or(repo).to_string(),
        version: tag,
        author: rel
            .pointer("/author/login")
            .and_then(Value::as_str)
            .map(str::to_string),
        game: GameHint::default(),
        files,
        deps: vec![],
        page_url: s(rel, "html_url"),
        manual_only: None,
        licence: None,
    })
}

impl ModProvider for Github {
    fn id(&self) -> ProviderId {
        ProviderId::Github
    }

    fn resolve<'a>(
        &'a self,
        fetch: &'a dyn Fetch,
        r: &'a ExternalRef,
        _ctx: &'a ResolveCtx,
    ) -> BoxFuture<'a, AppResult<Resolved>> {
        Box::pin(async move {
            let repo = r
                .project
                .as_deref()
                .ok_or_else(|| AppError::invalid("the GitHub link names no repository"))?;
            let rel = fetch
                .get_json(&release_url(repo, r.version.as_deref()))
                .await?;
            map(repo, &rel, r.extras.get("asset").map(String::as_str))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::provider::MockFetch;
    use serde_json::json;

    fn rel() -> Value {
        json!({"tag_name":"v1.0","html_url":"https://github.com/a/b/releases/tag/v1.0","author":{"login":"a"},
          "assets":[
            {"name":"notes.txt","size":1,"browser_download_url":"https://github.com/a/b/releases/download/v1.0/notes.txt"},
            {"name":"mod.zip","size":5,"digest":"sha256:abcd","browser_download_url":"https://github.com/a/b/releases/download/v1.0/mod.zip"},
            {"name":"mod2.jar","size":6,"browser_download_url":"https://github.com/a/b/releases/download/v1.0/mod2.jar"}]})
    }

    #[test]
    fn filters_assets_and_reads_digest() {
        let got = map("a/b", &rel(), None).unwrap();
        assert_eq!(got.files.len(), 2);
        assert!(got.files[0].primary && !got.files[1].primary);
        assert_eq!(got.files[0].hashes.sha256.as_deref(), Some("abcd"));
        let one = map("a/b", &rel(), Some("mod2.jar")).unwrap();
        assert_eq!(one.files.len(), 1);
        assert!(map("a/b", &rel(), Some("missing.zip")).is_err());
    }

    #[tokio::test]
    async fn latest_release_through_fetch() {
        let mut f = MockFetch::default();
        f.answers.insert(release_url("a/b", None), rel());
        let r = ExternalRef {
            project: Some("a/b".into()),
            ..ExternalRef::new(ProviderId::Github, "u")
        };
        let got = Github
            .resolve(&f, &r, &ResolveCtx::default())
            .await
            .unwrap();
        assert_eq!(got.version_id, "v1.0");
    }
}
