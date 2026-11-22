//! Download, verify, unpack, register; and the way back out.
//!
//! The pieces that touch the network ([`download`]) are thin and the rest takes
//! a file already on disk, so everything with logic in it is tested without a
//! socket. An external mod is an ordinary local mod (`Origin::External`) plus
//! a row in [`store`]: staging, deploy, the ledger and the journal treat it
//! like any other local mod, so uninstalling it is the local-mod removal and the
//! next sandbox deploy purges its files through the ledger.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};

use super::provider::{Fetch, ProviderAvailability, Registry, ResolveCtx};
use super::store::{self, ExternalRow};
use super::{parse_input, ExternalSource, Hashes, ProviderId, RemoteFile, Resolved};
use crate::download::{DownloadManager, DownloadRequest};
use crate::error::{AppError, AppResult};
use crate::library::LibraryDb;
use crate::local::store as lstore;
use crate::local::{NewLocalMod, Origin};

/// Bytes for a size setting in MB. `0` is unlimited, as everywhere else.
pub fn cap_bytes(max_mb: u64) -> u64 {
    max_mb.saturating_mul(1024 * 1024)
}

/// Refuse loudly when a declared size is over the cap. `cap == 0` never refuses.
pub fn check_size_cap(declared: Option<u64>, cap: u64, what: &str) -> AppResult<()> {
    match declared {
        Some(n) if cap > 0 && n > cap => Err(AppError::invalid(format!(
            "{what} is {} MB, over the {} MB download limit in Settings (set it to 0 for no limit)",
            n / (1024 * 1024),
            cap / (1024 * 1024)
        ))),
        _ => Ok(()),
    }
}

/// How a finished download was checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verified {
    Sha512,
    Sha256,
    /// The provider gave only a size (Thunderstore) or only a digest this app
    /// cannot compute (sha1), and the size matched.
    Size,
    /// The provider gave nothing to check against (a direct link).
    None,
}

impl Verified {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha512 => "sha512",
            Self::Sha256 => "sha256",
            Self::Size => "size",
            Self::None => "none",
        }
    }
}

fn digest<D: Digest>(path: &Path) -> AppResult<String> {
    let mut f = std::fs::File::open(path)?;
    let mut d = D::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        d.update(&buf[..n]);
    }
    Ok(hex::encode(d.finalize()))
}

/// Compare a file to what the provider published, strongest digest first. A
/// mismatch is an error, never a warning, and the caller deletes the file.
pub fn verify_file(
    path: &Path,
    size: Option<u64>,
    hashes: &Hashes,
    cap: u64,
) -> AppResult<Verified> {
    let actual = std::fs::metadata(path)?.len();
    check_size_cap(Some(actual), cap, "the downloaded file")?;
    if let Some(want) = size {
        if want != actual {
            return Err(AppError::invalid(format!(
                "the download is {actual} bytes but the provider says {want}; it was discarded"
            )));
        }
    }
    let want = |h: &Option<String>| h.as_ref().map(|x| x.trim().to_ascii_lowercase());
    if let Some(w) = want(&hashes.sha512) {
        return if digest::<Sha512>(path)? == w {
            Ok(Verified::Sha512)
        } else {
            Err(AppError::invalid(
                "the download does not match the provider's sha512; it was discarded",
            ))
        };
    }
    if let Some(w) = want(&hashes.sha256) {
        return if digest::<Sha256>(path)? == w {
            Ok(Verified::Sha256)
        } else {
            Err(AppError::invalid(
                "the download does not match the provider's sha256; it was discarded",
            ))
        };
    }
    Ok(if size.is_some() {
        Verified::Size
    } else {
        Verified::None
    })
}

/// A provider-supplied file name made safe to create: one component, no dots-only.
pub fn safe_file_name(raw: &str) -> AppResult<String> {
    let n = raw.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    if n.is_empty() || n == "." || n == ".." || n.contains('\0') {
        return Err(AppError::invalid(format!(
            "the provider named a file {raw:?}, which cannot be saved"
        )));
    }
    Ok(n.to_string())
}

/// Download one file into a fresh working directory under `work_root` and
/// check it. Returns the verified path; on any failure the directory is gone.
pub async fn download(
    dm: &DownloadManager,
    work_root: &Path,
    resolved: &Resolved,
    file: &RemoteFile,
    cap: u64,
) -> AppResult<(PathBuf, Verified)> {
    check_size_cap(file.size, cap, &file.name)?;
    let name = safe_file_name(&file.name)?;
    let dir = work_root
        .join(format!(
            "dl-{}-{}",
            resolved.provider.as_str(),
            std::process::id()
        ))
        .join(&hex::encode(Sha256::digest(file.url.as_bytes()))[..12]);
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(&name);
    let outcome = async {
        dm.run_to_completion(DownloadRequest {
            id: format!("ext:{}:{}", resolved.provider.as_str(), resolved.version_id),
            url: file.url.clone(),
            dest: dest.clone(),
            label: format!("{} {}", resolved.title, resolved.version)
                .trim()
                .to_string(),
            sha256: file.hashes.sha256.clone(),
            size_hint: file.size,
            priority: 0,
            limit_bps: None,
            meta: Default::default(),
        })
        .await?;
        verify_file(&dest, file.size, &file.hashes, cap)
    }
    .await;
    match outcome {
        Ok(v) => Ok((dest, v)),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

/// Where the result goes.
#[derive(Debug, Clone, Default)]
pub struct Target {
    pub app_id: Option<i64>,
    pub app_slug: Option<String>,
}

/// What the game's rules say about where and what.
pub struct InstallEnv<'a> {
    pub db: &'a LibraryDb,
    /// `paths.local_mods_dir()`.
    pub store_root: &'a Path,
    pub mod_targets: &'a [String],
    pub mod_extensions: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub local_id: i64,
    pub name: String,
    pub verified: Verified,
    pub files: usize,
}

/// Register a file that is already on disk and verified. Row first, then the
/// files, rolled back together, the same order as a dropped import.
pub fn install_downloaded(
    env: &InstallEnv<'_>,
    resolved: &Resolved,
    source_url: &str,
    path: &Path,
    verified: Verified,
    target: &Target,
) -> AppResult<Installed> {
    let prepared = lstore::prepare(
        env.store_root,
        path,
        &lstore::ImportOptions {
            mod_targets: env.mod_targets,
            mod_extensions: env.mod_extensions,
            payload: None,
            rel_path: None,
        },
    )?;
    let name = if resolved.title.trim().is_empty() {
        prepared.name.clone()
    } else {
        resolved.title.clone()
    };
    let created = env.db.local_create(&NewLocalMod {
        name: name.clone(),
        app_id: target.app_id,
        app_slug: target.app_slug.clone(),
        version: Some(resolved.version.clone()).filter(|v| !v.is_empty()),
        author: resolved.author.clone(),
        origin: Origin::External,
        origin_label: Some(format!("{} - {}", resolved.provider.label(), source_url)),
        rel_path: prepared.rel_path.clone(),
        source: None,
        files: prepared.files as i64,
        bytes: prepared.bytes as i64,
    });
    let id = match created {
        Ok(id) => id,
        Err(e) => {
            lstore::discard(env.store_root);
            return Err(e);
        }
    };
    let row = ExternalRow {
        local_id: id,
        source: ExternalSource {
            provider: resolved.provider,
            url: source_url.to_string(),
            project_id: resolved.project_id.clone(),
            version_id: resolved.version_id.clone(),
            version: resolved.version.clone(),
        },
        verified: verified.as_str().to_string(),
    };
    if let Err(e) =
        lstore::commit(env.store_root, &prepared, id).and_then(|()| store::put(env.db, &row))
    {
        let _ = env.db.local_delete(id);
        lstore::remove(env.store_root, id);
        return Err(e);
    }
    Ok(Installed {
        local_id: id,
        name,
        verified,
        files: prepared.files,
    })
}

/// Remove an external mod: its row, its files, and (through the next deploy's
/// ledger purge) its place in every sandbox. Refuses a mod that did not come
/// from a link, so this cannot be used to delete someone's dropped import.
/// Returns the sandboxes that held it and need a redeploy.
pub fn remove(db: &LibraryDb, store_root: &Path, local_id: i64) -> AppResult<Vec<i64>> {
    if store::get(db, local_id)?.is_none() {
        return Err(AppError::invalid("that mod was not added from a link"));
    }
    let affected = db.local_delete(local_id)?;
    store::delete(db, local_id)?;
    lstore::remove(store_root, local_id);
    Ok(affected)
}

/// What `add` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AddOutcome {
    Installed(Installed),
    /// The provider does not allow or cannot do the download here; show the page.
    OpenPage {
        url: String,
        reason: String,
    },
    /// The provider is switched off.
    Unavailable {
        provider: ProviderId,
        reason: String,
    },
}

/// THE ENTRY POINT for `tmc mods add <url>` and for a paste or drop in the app:
/// text in, installed mod (or an honest "open the page" / "not available") out.
///
/// The caller supplies the services; this performs no I/O of its own beyond
/// them. A link that arrived from the OS must go through
/// [`super::pending::route`] first, never straight here.
#[allow(clippy::too_many_arguments)]
pub async fn add(
    text: &str,
    fetch: &dyn Fetch,
    registry: &Registry,
    avail: &ProviderAvailability,
    ctx: &ResolveCtx,
    dm: &DownloadManager,
    work_root: &Path,
    max_download_mb: u64,
    env: &InstallEnv<'_>,
    target: &Target,
) -> AppResult<AddOutcome> {
    let r = parse_input(text).map_err(|e| AppError::invalid(e.to_string()))?;
    let provider = registry
        .get(r.provider)
        .ok_or_else(|| AppError::invalid("no provider handles that link"))?;
    let state = provider.credential_state(avail);
    if let super::CredentialState::NotConfigured { reason } = state {
        return Ok(AddOutcome::Unavailable {
            provider: r.provider,
            reason,
        });
    }
    let resolved = provider.resolve(fetch, &r, ctx).await?;
    if let Some(url) = &resolved.manual_only {
        return Ok(AddOutcome::OpenPage {
            url: url.clone(),
            reason: format!(
                "{} items are installed from the {} page",
                resolved.provider.label(),
                resolved.provider.label()
            ),
        });
    }
    let file = resolved
        .primary_file()
        .ok_or_else(|| AppError::invalid("that version has no file to download"))?
        .clone();
    let cap = cap_bytes(max_download_mb);
    let (path, verified) = download(dm, work_root, &resolved, &file, cap).await?;
    let installed = install_downloaded(env, &resolved, &r.url, &path, verified, target);
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
    Ok(AddOutcome::Installed(installed?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::tree::testutil::write_zip;
    use crate::external::GameHint;
    use sha2::Digest;

    fn resolved() -> Resolved {
        Resolved {
            provider: ProviderId::Modrinth,
            project_id: "P".into(),
            version_id: "V1".into(),
            title: "Sodium".into(),
            version: "0.5".into(),
            author: Some("jelly".into()),
            game: GameHint::default(),
            files: vec![],
            deps: vec![],
            page_url: None,
            manual_only: None,
            licence: None,
        }
    }

    #[test]
    fn size_cap_zero_is_unlimited_and_overrun_is_loud() {
        assert!(check_size_cap(Some(u64::MAX), 0, "x").is_ok());
        assert!(check_size_cap(None, 10, "x").is_ok());
        let e = check_size_cap(Some(5 * 1024 * 1024), cap_bytes(1), "pack.zip").unwrap_err();
        assert!(e.detail().contains("pack.zip") && e.detail().contains("limit"));
        assert_eq!(cap_bytes(2), 2 * 1024 * 1024);
    }

    #[test]
    fn verify_prefers_sha512_and_fails_on_mismatch() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.bin");
        std::fs::write(&p, b"hello").unwrap();
        let s512 = hex::encode(Sha512::digest(b"hello"));
        let s256 = hex::encode(Sha256::digest(b"hello"));
        let h = |a: Option<&str>, b: Option<&str>| Hashes {
            sha512: a.map(Into::into),
            sha256: b.map(Into::into),
            ..Default::default()
        };
        assert_eq!(
            verify_file(&p, Some(5), &h(Some(&s512.to_uppercase()), None), 0).unwrap(),
            Verified::Sha512
        );
        assert_eq!(
            verify_file(&p, None, &h(None, Some(&s256)), 0).unwrap(),
            Verified::Sha256
        );
        assert_eq!(
            verify_file(&p, Some(5), &Hashes::default(), 0).unwrap(),
            Verified::Size
        );
        assert_eq!(
            verify_file(&p, None, &Hashes::default(), 0).unwrap(),
            Verified::None
        );
        assert!(
            verify_file(&p, None, &h(Some("00"), Some(&s256)), 0).is_err(),
            "a wrong sha512 is not rescued by a right sha256"
        );
        assert!(verify_file(&p, Some(6), &Hashes::default(), 0).is_err());
        assert!(
            verify_file(&p, None, &Hashes::default(), 3).is_err(),
            "over the cap after download"
        );
    }

    #[test]
    fn file_names_are_reduced_to_one_component() {
        assert_eq!(safe_file_name("../../etc/x.zip").unwrap(), "x.zip");
        assert_eq!(safe_file_name("a\\b.jar").unwrap(), "b.jar");
        assert!(safe_file_name("..").is_err());
        assert!(safe_file_name("").is_err());
    }

    #[test]
    fn install_registers_an_external_row_and_remove_undoes_it() {
        let work = tempfile::tempdir().unwrap();
        let store_root = work.path().join("store");
        std::fs::create_dir_all(&store_root).unwrap();
        let z = work.path().join("pack.zip");
        write_zip(&z, &[("mods/a.txt", "x")]);
        let db = LibraryDb::open_memory().unwrap();
        let env = InstallEnv {
            db: &db,
            store_root: &store_root,
            mod_targets: &[],
            mod_extensions: &[],
        };
        let got = install_downloaded(
            &env,
            &resolved(),
            "https://modrinth.com/mod/sodium",
            &z,
            Verified::Sha512,
            &Target::default(),
        )
        .unwrap();

        let row = store::get(&db, got.local_id).unwrap().unwrap();
        assert_eq!(
            (
                row.source.provider,
                row.source.version_id.as_str(),
                row.verified.as_str()
            ),
            (ProviderId::Modrinth, "V1", "sha512")
        );
        let local = db.local_get(got.local_id).unwrap().unwrap();
        assert_eq!(
            (
                local.origin,
                local.version.as_deref(),
                local.author.as_deref()
            ),
            (Origin::External, Some("0.5"), Some("jelly"))
        );
        assert!(lstore::local_root(&store_root, got.local_id).exists());
        assert_eq!(store::list(&db).unwrap().len(), 1);

        remove(&db, &store_root, got.local_id).unwrap();
        assert!(store::get(&db, got.local_id).unwrap().is_none());
        assert!(db.local_get(got.local_id).unwrap().is_none());
        assert!(!lstore::local_root(&store_root, got.local_id).exists());
    }

    #[test]
    fn remove_refuses_a_mod_that_is_not_external() {
        let work = tempfile::tempdir().unwrap();
        let db = LibraryDb::open_memory().unwrap();
        let id = db
            .local_create(&NewLocalMod {
                name: "mine".into(),
                app_id: None,
                app_slug: None,
                version: None,
                author: None,
                origin: Origin::Dropped,
                origin_label: None,
                rel_path: String::new(),
                source: None,
                files: 0,
                bytes: 0,
            })
            .unwrap();
        assert!(remove(&db, work.path(), id).is_err());
        assert!(db.local_get(id).unwrap().is_some(), "left alone");
    }
}
