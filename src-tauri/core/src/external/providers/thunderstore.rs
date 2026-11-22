//! Thunderstore. No key. The API gives a download URL and a size but no hash,
//! so verification is by size; the manifest inside the zip is read afterwards.

use futures_util::future::BoxFuture;
use serde_json::Value;

use super::s;
use crate::error::{AppError, AppResult};
use crate::external::provider::{Fetch, ModProvider, ResolveCtx};
use crate::external::{
    DepKind, DepRef, ExternalRef, GameHint, Hashes, ProviderId, RemoteFile, Resolved,
};

pub struct Thunderstore;

pub(crate) fn package_url(project: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("https://thunderstore.io/api/experimental/package/{project}/{v}/"),
        None => format!("https://thunderstore.io/api/experimental/package/{project}/"),
    }
}

/// `Owner-Name-1.2.3` to a dependency.
fn dep(raw: &str) -> Option<DepRef> {
    let (rest, ver) = raw.rsplit_once('-')?;
    let (owner, name) = rest.split_once('-')?;
    Some(DepRef {
        project: format!("{owner}/{name}"),
        version: Some(ver.to_string()),
        kind: DepKind::Required,
    })
}

/// `body` is either the package (with a `latest`) or one version.
pub fn map(project: &str, body: &Value) -> AppResult<Resolved> {
    let v = body.get("latest").unwrap_or(body);
    let url = s(v, "download_url")
        .ok_or_else(|| AppError::invalid("Thunderstore sent no download link"))?;
    let version = s(v, "version_number").unwrap_or_default();
    let name = s(v, "full_name").unwrap_or_else(|| project.replace('/', "-"));
    Ok(Resolved {
        provider: ProviderId::Thunderstore,
        project_id: project.to_string(),
        version_id: version.clone(),
        title: s(body, "name")
            .or_else(|| s(v, "name"))
            .unwrap_or_else(|| project.to_string()),
        version: version.clone(),
        author: s(body, "owner").or_else(|| s(v, "namespace")),
        game: GameHint::default(),
        files: vec![RemoteFile {
            url,
            name: format!("{name}-{version}.zip"),
            size: v.get("file_size").and_then(Value::as_u64),
            hashes: Hashes::default(),
            primary: true,
        }],
        deps: v
            .get("dependencies")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|d| d.as_str().and_then(dep)).collect())
            .unwrap_or_default(),
        page_url: s(body, "package_url"),
        manual_only: None,
        licence: None,
    })
}

impl ModProvider for Thunderstore {
    fn id(&self) -> ProviderId {
        ProviderId::Thunderstore
    }

    fn resolve<'a>(
        &'a self,
        fetch: &'a dyn Fetch,
        r: &'a ExternalRef,
        _ctx: &'a ResolveCtx,
    ) -> BoxFuture<'a, AppResult<Resolved>> {
        Box::pin(async move {
            let p = r
                .project
                .as_deref()
                .ok_or_else(|| AppError::invalid("the Thunderstore link names no package"))?;
            let body = fetch
                .get_json(&package_url(p, r.version.as_deref()))
                .await?;
            let mut got = map(p, &body)?;
            got.game.game = r.game.clone();
            Ok(got)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::provider::MockFetch;
    use serde_json::json;

    #[tokio::test]
    async fn resolves_latest_and_pins_a_version() {
        let latest = json!({"name":"Pkg","owner":"Owner","package_url":"https://thunderstore.io/package/Owner/Pkg/",
          "latest":{"version_number":"1.2.3","full_name":"Owner-Pkg","file_size":99,
            "download_url":"https://thunderstore.io/package/download/Owner/Pkg/1.2.3/",
            "dependencies":["bbepis-BepInExPack-5.4.2100"]}});
        let mut f = MockFetch::default();
        f.answers.insert(package_url("Owner/Pkg", None), latest);
        let r = ExternalRef {
            project: Some("Owner/Pkg".into()),
            game: Some("ror2".into()),
            ..ExternalRef::new(ProviderId::Thunderstore, "u")
        };
        let got = Thunderstore
            .resolve(&f, &r, &ResolveCtx::default())
            .await
            .unwrap();
        assert_eq!(
            (got.version.as_str(), got.files[0].size),
            ("1.2.3", Some(99))
        );
        assert_eq!(got.deps[0].project, "bbepis/BepInExPack");
        assert_eq!(got.deps[0].version.as_deref(), Some("5.4.2100"));
        assert_eq!(got.game.game.as_deref(), Some("ror2"));
        assert!(got.files[0].hashes.is_empty());
    }

    #[test]
    fn missing_download_url_fails() {
        assert!(map("a/b", &serde_json::json!({"latest":{"version_number":"1"}})).is_err());
    }
}
