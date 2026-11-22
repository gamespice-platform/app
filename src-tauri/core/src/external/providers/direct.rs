//! A bare link to a file. Nothing to look up; the link is the file. A
//! `#sha256=<hex>` fragment, when present, is honoured as the expected digest.

use futures_util::future::BoxFuture;

use crate::error::{AppError, AppResult};
use crate::external::provider::{Fetch, ModProvider, ResolveCtx};
use crate::external::{ExternalRef, GameHint, Hashes, ProviderId, RemoteFile, Resolved};

pub struct Direct;

pub fn resolve_direct(r: &ExternalRef) -> AppResult<Resolved> {
    let u = url::Url::parse(&r.url).map_err(|_| AppError::invalid("that is not a valid link"))?;
    let name = r
        .extras
        .get("file")
        .cloned()
        .filter(|n| !n.is_empty())
        .or_else(|| {
            u.path_segments()
                .and_then(|mut s| s.next_back())
                .map(str::to_string)
        })
        .ok_or_else(|| AppError::invalid("the link names no file"))?;
    let sha256 = u
        .fragment()
        .and_then(|f| f.strip_prefix("sha256="))
        .map(str::to_ascii_lowercase);
    let mut bare = u.clone();
    bare.set_fragment(None);
    let host = u.host_str().unwrap_or("the web").to_string();
    Ok(Resolved {
        provider: ProviderId::Direct,
        project_id: bare.to_string(),
        version_id: bare.to_string(),
        title: name
            .rsplit_once('.')
            .map(|(b, _)| b)
            .unwrap_or(&name)
            .to_string(),
        version: String::new(),
        author: Some(host),
        game: GameHint::default(),
        files: vec![RemoteFile {
            url: bare.to_string(),
            name,
            size: None,
            hashes: Hashes {
                sha256,
                ..Hashes::default()
            },
            primary: true,
        }],
        deps: vec![],
        page_url: None,
        manual_only: None,
        licence: None,
    })
}

impl ModProvider for Direct {
    fn id(&self) -> ProviderId {
        ProviderId::Direct
    }

    fn resolve<'a>(
        &'a self,
        _fetch: &'a dyn Fetch,
        r: &'a ExternalRef,
        _ctx: &'a ResolveCtx,
    ) -> BoxFuture<'a, AppResult<Resolved>> {
        Box::pin(async move { resolve_direct(r) })
    }

    /// A direct link has no notion of "newer".
    fn latest<'a>(
        &'a self,
        _f: &'a dyn Fetch,
        _i: &'a crate::external::ExternalSource,
        _c: &'a ResolveCtx,
    ) -> BoxFuture<'a, AppResult<Option<Resolved>>> {
        Box::pin(async { Ok(None) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::parse_input;

    #[test]
    fn names_the_file_and_reads_the_fragment() {
        let r = parse_input("https://example.com/dl/My.Mod.zip#sha256=ABCD").unwrap();
        let got = resolve_direct(&r).unwrap();
        assert_eq!(got.files[0].name, "My.Mod.zip");
        assert_eq!(got.files[0].hashes.sha256.as_deref(), Some("abcd"));
        assert!(!got.files[0].url.contains('#'));
        assert_eq!(got.title, "My.Mod");
    }
}
