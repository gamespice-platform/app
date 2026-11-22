//! Modrinth, API v2. No key; an identifying `User-Agent` is mandatory and set
//! by [`HttpFetch`](crate::external::provider::HttpFetch). Files carry sha1 and sha512.

use futures_util::future::BoxFuture;
use serde_json::Value;

use super::{s, strs};
use crate::error::{AppError, AppResult};
use crate::external::provider::{Fetch, ModProvider, ResolveCtx};
use crate::external::{
    DepKind, DepRef, ExternalRef, GameHint, Hashes, ProviderId, RemoteFile, Resolved,
};

const API: &str = "https://api.modrinth.com/v2";

pub struct Modrinth;

fn enc(v: &str) -> String {
    url::form_urlencoded::byte_serialize(v.as_bytes()).collect()
}

pub(crate) fn project_url(slug: &str) -> String {
    format!("{API}/project/{}", enc(slug))
}

pub(crate) fn version_url(id: &str) -> String {
    format!("{API}/version/{}", enc(id))
}

/// Newest-first list, narrowed by loader and game version when the caller has them.
pub(crate) fn versions_url(slug: &str, ctx: &ResolveCtx) -> String {
    let mut u = format!("{API}/project/{}/version", enc(slug));
    let mut q = Vec::new();
    if let Some(l) = &ctx.loader {
        q.push(format!("loaders={}", enc(&format!("[\"{l}\"]"))));
    }
    if let Some(g) = &ctx.game_version {
        q.push(format!("game_versions={}", enc(&format!("[\"{g}\"]"))));
    }
    if !q.is_empty() {
        u.push('?');
        u.push_str(&q.join("&"));
    }
    u
}

/// Map a project and one of its versions. Pure.
pub fn map_version(project: &Value, version: &Value) -> AppResult<Resolved> {
    let files: Vec<RemoteFile> = version
        .get("files")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    Some(RemoteFile {
                        url: s(f, "url")?,
                        name: s(f, "filename")?,
                        size: f.get("size").and_then(Value::as_u64),
                        hashes: Hashes {
                            sha512: f
                                .pointer("/hashes/sha512")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            sha1: f
                                .pointer("/hashes/sha1")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            ..Hashes::default()
                        },
                        primary: f.get("primary").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if files.is_empty() {
        return Err(AppError::invalid(
            "that Modrinth version has no files to download",
        ));
    }
    let deps = version
        .get("dependencies")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|d| {
                    let kind = match d.get("dependency_type")?.as_str()? {
                        "required" => DepKind::Required,
                        "optional" => DepKind::Optional,
                        "incompatible" => DepKind::Incompatible,
                        "embedded" => DepKind::Embedded,
                        _ => return None,
                    };
                    Some(DepRef {
                        project: s(d, "project_id")?,
                        version: s(d, "version_id"),
                        kind,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let slug = s(project, "slug")
        .or_else(|| s(project, "id"))
        .unwrap_or_default();
    let class = s(project, "project_type").unwrap_or_else(|| "mod".into());
    Ok(Resolved {
        provider: ProviderId::Modrinth,
        project_id: s(project, "id")
            .or_else(|| s(version, "project_id"))
            .unwrap_or_else(|| slug.clone()),
        version_id: s(version, "id")
            .ok_or_else(|| AppError::invalid("Modrinth sent a version with no id"))?,
        title: s(project, "title").unwrap_or_else(|| slug.clone()),
        version: s(version, "version_number").unwrap_or_default(),
        author: None,
        game: GameHint {
            game: Some("minecraft".into()),
            loaders: strs(version, "loaders"),
            game_versions: strs(version, "game_versions"),
        },
        files,
        deps,
        page_url: Some(format!("https://modrinth.com/{class}/{slug}")),
        manual_only: None,
        licence: project
            .pointer("/license/id")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

impl ModProvider for Modrinth {
    fn id(&self) -> ProviderId {
        ProviderId::Modrinth
    }

    fn resolve<'a>(
        &'a self,
        fetch: &'a dyn Fetch,
        r: &'a ExternalRef,
        ctx: &'a ResolveCtx,
    ) -> BoxFuture<'a, AppResult<Resolved>> {
        Box::pin(async move {
            let slug = r
                .project
                .as_deref()
                .ok_or_else(|| AppError::invalid("the Modrinth link names no project"))?;
            let project = fetch.get_json(&project_url(slug)).await?;
            let version = if let Some(v) = &r.version {
                fetch.get_json(&version_url(v)).await?
            } else {
                let list = fetch.get_json(&versions_url(slug, ctx)).await?;
                let arr = list.as_array().cloned().unwrap_or_default();
                // Prefer a release; fall back to the newest of anything.
                arr.iter()
                    .find(|v| v.get("version_type").and_then(Value::as_str) == Some("release"))
                    .or(arr.first())
                    .cloned()
                    .ok_or_else(|| {
                        AppError::invalid(
                            "Modrinth has no version of that mod for this game version and loader",
                        )
                    })?
            };
            map_version(&project, &version)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::provider::MockFetch;
    use serde_json::json;

    pub(crate) fn project() -> Value {
        json!({"id":"AANobbMI","slug":"sodium","title":"Sodium","project_type":"mod","license":{"id":"LGPL-3.0-only"}})
    }
    pub(crate) fn version(id: &str, ty: &str) -> Value {
        json!({"id":id,"project_id":"AANobbMI","version_number":"0.5.8","version_type":ty,
          "loaders":["fabric"],"game_versions":["1.20.1"],
          "dependencies":[{"project_id":"P1","version_id":null,"dependency_type":"required"}],
          "files":[{"url":"https://cdn.modrinth.com/data/x/sodium.jar","filename":"sodium.jar","size":10,
                    "primary":true,"hashes":{"sha1":"aa","sha512":"bb"}}]})
    }

    #[tokio::test]
    async fn resolves_latest_release_with_filters() {
        let ctx = ResolveCtx {
            loader: Some("fabric".into()),
            game_version: Some("1.20.1".into()),
        };
        let mut f = MockFetch::default();
        f.answers.insert(project_url("sodium"), project());
        f.answers.insert(
            versions_url("sodium", &ctx),
            json!([version("beta1", "beta"), version("rel1", "release")]),
        );
        let r = ExternalRef {
            project: Some("sodium".into()),
            ..ExternalRef::new(ProviderId::Modrinth, "u")
        };
        let got = Modrinth.resolve(&f, &r, &ctx).await.unwrap();
        assert_eq!(got.version_id, "rel1");
        assert_eq!(got.title, "Sodium");
        assert_eq!(got.files[0].hashes.sha512.as_deref(), Some("bb"));
        assert_eq!(got.game.loaders, vec!["fabric"]);
        assert_eq!(got.deps.len(), 1);
        assert!(versions_url("sodium", &ctx).contains("loaders=%5B%22fabric%22%5D"));
    }

    #[tokio::test]
    async fn pinned_version_and_empty_list_fail_loudly() {
        let mut f = MockFetch::default();
        f.answers.insert(project_url("sodium"), project());
        f.answers
            .insert(version_url("v9"), version("v9", "release"));
        f.answers
            .insert(versions_url("sodium", &ResolveCtx::default()), json!([]));
        let mut r = ExternalRef {
            project: Some("sodium".into()),
            ..ExternalRef::new(ProviderId::Modrinth, "u")
        };
        assert!(Modrinth
            .resolve(&f, &r, &ResolveCtx::default())
            .await
            .is_err());
        r.version = Some("v9".into());
        assert_eq!(
            Modrinth
                .resolve(&f, &r, &ResolveCtx::default())
                .await
                .unwrap()
                .version_id,
            "v9"
        );
    }

    #[test]
    fn version_without_files_is_refused() {
        let mut v = version("a", "release");
        v["files"] = json!([]);
        assert!(map_version(&project(), &v).is_err());
    }
}
