//! **Exporting a sandbox as a file**: a Modrinth `.mrpack`, or a plain TMC
//! share zip.
//!
//! [`super::share`] explains why the pasteable code carries item ids and not
//! files. This is the other half: somebody who wants the FILES (to hand a pack
//! to a launcher that reads `.mrpack`, or to keep one) gets them, written by
//! this side into the downloads folder.
//!
//! THE WEBVIEW NAMES NOTHING
//! -------------------------
//! The core functions take the folder to write into, and the command passes the
//! one the app already resolves for "a file the user asked for"
//! (`AppState::download_target_dir`, the jail's `downloads` anchor). No path
//! from the webview exists to be abused, and the file name is built here from
//! the sandbox's name, reduced to a safe single component, and made unique so
//! an export never overwrites an existing file.
//!
//! WHAT AN `.mrpack` HOLDS HERE, AND WHY NO `files` ENTRY
//! ------------------------------------------------------
//! The format's `files` array names downloads, and a download must come from
//! one of the hosts the format permits (`local::modpack::ALLOWED_HOSTS`; the
//! importer refuses anything else, and so does every other launcher). A mod
//! staged from this site has no such URL, so listing one would produce a pack
//! the importer, correctly, rejects. Everything therefore travels in
//! `overrides/`, the way Modrinth's own "include files" option does for a file
//! with no public URL. The winner of each path is the merge tree's, so a pack
//! exported from a sandbox is the folder the game would see after a deploy.
//! Nothing in the produced file asks its reader to fetch from anywhere.
//!
//! WHAT IS LEFT OUT
//! ----------------
//! Mods not staged yet (no files to give), disabled mods, symbolic links, and
//! everything about this machine (game folder, launch environment, ledger).
//! The report says what was skipped so the UI can show it.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::deploy::merge::{self, DeployMod};
use crate::error::{AppError, AppResult};
use crate::library::sandbox::Sandbox;
use crate::library::share;
use crate::plugins::jail::join_relative;

/// Most bytes one export will pack, the same ceiling an import has.
pub const MAX_EXPORT_BYTES: u64 = crate::local::store::MAX_IMPORT_BYTES;

/// What an export did. Names a file, never a path: the UI says "downloads
/// folder".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    /// File name inside the downloads folder.
    pub file_name: String,
    pub files: usize,
    pub bytes: u64,
    /// Things left out, each a sentence.
    pub notes: Vec<String>,
}

/// A file name stem from a sandbox's name: alphanumerics, space, `-`, `_`;
/// everything else becomes `_`; trimmed; never empty.
fn safe_stem(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();

    let trimmed = mapped.trim_matches(|c: char| c == ' ' || c == '_');

    if trimmed.is_empty() {
        "sandbox".into()
    } else {
        trimmed.to_string()
    }
}

/// An open `<name>.tmcpart` that did not exist, and the final path it will be
/// renamed to. `create_new` is the guard: a taken name (or a planted link) is
/// skipped, never written through.
fn reserve(dir: &Path, stem: &str, ext: &str) -> AppResult<(std::fs::File, PathBuf, PathBuf)> {
    std::fs::create_dir_all(dir)?;

    for n in 0..1000u32 {
        let name = if n == 0 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem} ({}).{ext}", n + 1)
        };

        let final_path = dir.join(&name);

        if final_path.symlink_metadata().is_ok() {
            continue;
        }

        let part = dir.join(format!("{name}.tmcpart"));

        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
        {
            Ok(file) => return Ok((file, part, final_path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }

    Err(AppError::invalid(
        "There are too many exports with that name in the downloads folder.",
    ))
}

fn finish(part: &Path, final_path: &Path) -> AppResult<String> {
    // Never over an existing file: a name that appeared meanwhile fails.
    if final_path.symlink_metadata().is_ok() {
        let _ = std::fs::remove_file(part);

        return Err(AppError::invalid("That export file already exists."));
    }

    std::fs::rename(part, final_path)?;

    Ok(final_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string())
}

fn index_json(sandbox: &Sandbox, notes: &mut Vec<String>) -> AppResult<String> {
    let Some(game_version) = sandbox
        .game_version
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Err(AppError::invalid(
            "Set the sandbox's game version first: a modpack has to say which Minecraft it is for.",
        ));
    };

    let mut dependencies = BTreeMap::new();
    dependencies.insert("minecraft".to_string(), game_version.to_string());

    // `.mrpack` names a loader by its dependency key and wants a version. A
    // made-up one would be read as a promise, so a loader with no recorded
    // version (or one the format has no key for) is left out and said so.
    if let Some(loader) = sandbox.loader.as_deref().filter(|l| !l.trim().is_empty()) {
        let key = match loader {
            "fabric" => Some("fabric-loader"),
            "quilt" => Some("quilt-loader"),
            "forge" => Some("forge"),
            "neoforge" => Some("neoforge"),
            _ => None,
        };

        match (key, sandbox.loader_version.as_deref().filter(|v| !v.trim().is_empty())) {
            (Some(key), Some(version)) => {
                dependencies.insert(key.to_string(), version.trim().to_string());
            }
            (Some(_), None) => notes.push(format!(
                "The loader ({loader}) has no recorded version, so the pack does not name one."
            )),
            (None, _) => notes.push(format!(
                "The loader ({loader}) has no .mrpack dependency key, so the pack does not name it."
            )),
        }
    }

    let index = serde_json::json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "1.0.0",
        "name": sandbox.name.trim().chars().take(200).collect::<String>(),
        "summary": sandbox.description.as_deref().unwrap_or_default().chars().take(500).collect::<String>(),
        "files": [],
        "dependencies": dependencies,
    });

    serde_json::to_string_pretty(&index)
        .map_err(|e| AppError::internal(format!("could not write the pack index: {e}")))
}

fn opts() -> zip::write::FileOptions<'static, 'static, ()> {
    zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true)
}

fn zip_err(e: zip::result::ZipError) -> AppError {
    AppError::internal(format!("could not write the export: {e}"))
}

/// Write `sandbox` as `<name>.mrpack` in `out_dir`.
///
/// `mods` is [`crate::library::deploy::deployable`]'s answer: the enabled,
/// staged mods with the folder each reads from.
pub fn export_mrpack(
    sandbox: &Sandbox,
    mods: &[DeployMod],
    out_dir: &Path,
) -> AppResult<ExportReport> {
    if sandbox.app_slug.as_deref() != Some("minecraft") {
        return Err(AppError::invalid(
            "Only a Minecraft sandbox can be exported as a modpack.",
        ));
    }

    let mut notes = Vec::new();
    let index = index_json(sandbox, &mut notes)?;

    let tree = merge::build(mods)?;
    notes.extend(tree.skipped.iter().cloned());

    for name in &tree.empty {
        notes.push(format!("{name} has no files on this device."));
    }

    let unstaged = sandbox
        .enabled_mods()
        .filter(|m| m.staged_at.is_none())
        .count();

    if unstaged > 0 {
        notes.push(format!(
            "{unstaged} mod(s) are not downloaded yet and were left out."
        ));
    }

    // Known before anything is written, so a too-large export leaves no file.
    let total: u64 = tree.files.values().map(|w| w.size).sum();

    if total > MAX_EXPORT_BYTES {
        return Err(AppError::invalid("That sandbox is too large to export."));
    }

    let (file, part, final_path) = reserve(out_dir, &safe_stem(&sandbox.name), "mrpack")?;

    let result = (|| -> AppResult<(usize, u64)> {
        let mut zip = zip::ZipWriter::new(file);

        zip.start_file("modrinth.index.json", opts())
            .map_err(zip_err)?;
        zip.write_all(index.as_bytes())?;

        let mut files = 0usize;
        let mut bytes = 0u64;

        for (rel, winner) in &tree.files {
            // The importer's own lexical gate, so whatever is written reads back.
            join_relative(Path::new("/x"), rel)?;

            let mut src = std::fs::File::open(&winner.source)?;

            zip.start_file(format!("overrides/{rel}"), opts())
                .map_err(zip_err)?;

            bytes += std::io::copy(
                &mut std::io::Read::by_ref(&mut src).take(winner.size),
                &mut zip,
            )?;
            files += 1;
        }

        zip.finish().map_err(zip_err)?;

        Ok((files, bytes))
    })();

    match result {
        Ok((files, bytes)) => Ok(ExportReport {
            file_name: finish(&part, &final_path)?,
            files,
            bytes,
            notes,
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&part);

            Err(e)
        }
    }
}

/// Write `sandbox` as a TMC share zip: the code, and the data behind it.
///
/// Item ids and settings only, exactly what [`share::export`] carries, so this
/// is no more a redistribution channel than the code is.
pub fn export_share_zip(sandbox: &Sandbox, out_dir: &Path) -> AppResult<ExportReport> {
    let json = serde_json::to_string_pretty(&share::to_shared(sandbox))
        .map_err(|e| AppError::internal(format!("could not encode the sandbox: {e}")))?;

    let code = share::export(sandbox)?;

    let (file, part, final_path) = reserve(out_dir, &safe_stem(&sandbox.name), "tmcshare.zip")?;

    let result = (|| -> AppResult<()> {
        let mut zip = zip::ZipWriter::new(file);

        zip.start_file("sandbox.json", opts()).map_err(zip_err)?;
        zip.write_all(json.as_bytes())?;

        zip.start_file("sandbox-code.txt", opts())
            .map_err(zip_err)?;
        zip.write_all(code.as_bytes())?;

        zip.finish().map_err(zip_err)?;

        Ok(())
    })();

    match result {
        Ok(()) => Ok(ExportReport {
            file_name: finish(&part, &final_path)?,
            files: 2,
            bytes: (json.len() + code.len()) as u64,
            notes: Vec::new(),
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&part);

            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::Strategy;
    use crate::library::sandbox::{Environment, SandboxMod};
    use crate::local::store::{prepare, ImportOptions};
    use sha2::{Digest, Sha512};

    fn sandbox(mods: Vec<SandboxMod>) -> Sandbox {
        Sandbox {
            id: 1,
            remote_id: None,
            app_id: 42,
            app_slug: Some("minecraft".into()),
            app_name: Some("Minecraft".into()),
            name: "My Pack: v2/../x".into(),
            description: Some("A pack".into()),
            environment: Environment::Client,
            strategy: Strategy::Hardlink,
            game_version: Some("1.20.1".into()),
            loader: Some("fabric".into()),
            loader_version: None,
            install_state: None,
            preset: None,
            is_default: true,
            cloud_sync: false,
            auto_update: false,
            game_dir: Some("/home/somebody/secret-place".into()),
            options: BTreeMap::new(),
            launch_args: Vec::new(),
            launch_env: BTreeMap::from([("HOME".into(), "/home/somebody".into())]),
            deployed_at: None,
            last_deploy: None,
            created_at: "x".into(),
            updated_at: "x".into(),
            mods,
        }
    }

    fn mod_row(key: &str, priority: i64) -> SandboxMod {
        SandboxMod {
            mod_key: key.into(),
            kind: "mod".into(),
            item_id: 1,
            name: key.into(),
            enabled: true,
            priority,
            release_id: None,
            version: None,
            staged_at: Some("x".into()),
            last_error: None,
        }
    }

    fn put(root: &Path, rel: &str, body: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("dirs");
        std::fs::write(p, body).expect("write");
    }

    fn sha(path: &Path) -> String {
        hex::encode(Sha512::digest(std::fs::read(path).expect("read")))
    }

    fn deploy_mod(key: &str, priority: i64, root: &Path) -> DeployMod {
        DeployMod {
            key: key.into(),
            name: key.into(),
            root: root.to_path_buf(),
            priority,
        }
    }

    #[test]
    fn an_exported_mrpack_imports_back_with_identical_hashes() {
        let tmp = tempfile::tempdir().expect("tmp");
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");

        put(&a, "mods/a.jar", b"jar-a");
        put(&a, "config/shared.toml", b"low");
        put(&b, "config/shared.toml", b"high-wins");
        put(&b, "mods/deep/b.jar", &vec![7u8; 100_000]);

        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", a.join("mods/link.jar")).expect("link");

        let row = sandbox(vec![mod_row("mod:1", 1), mod_row("mod:2", 5)]);
        let mods = vec![deploy_mod("mod:1", 1, &a), deploy_mod("mod:2", 5, &b)];
        let out = tmp.path().join("downloads");

        let report = export_mrpack(&row, &mods, &out).expect("export");
        let archive = out.join(&report.file_name);

        // Hostile characters in the name never reach the file name.
        assert!(
            !report.file_name.contains(['/', ':']),
            "{}",
            report.file_name
        );
        assert!(report.file_name.ends_with(".mrpack"));
        assert!(archive.is_file());
        assert_eq!(report.files, 3);
        assert!(!out.join(format!("{}.tmcpart", report.file_name)).exists());

        // The importer reads it as a pack and unpacks the overrides.
        let pack = crate::local::modpack::read(&archive)
            .expect("read")
            .expect("is a pack");

        assert_eq!(pack.game_version.as_deref(), Some("1.20.1"));
        assert!(pack.files.is_empty());

        let store = tmp.path().join("store");
        std::fs::create_dir_all(&store).expect("store");

        let prepared = prepare(
            &store,
            &archive,
            &ImportOptions {
                mod_targets: &[],
                mod_extensions: &[],
                payload: None,
                rel_path: None,
            },
        )
        .expect("prepare");

        let root = prepared.dir.join(&prepared.rel_path);

        assert_eq!(
            sha(&root.join("config/shared.toml")),
            sha(&b.join("config/shared.toml"))
        );
        assert_eq!(sha(&root.join("mods/a.jar")), sha(&a.join("mods/a.jar")));
        assert_eq!(
            sha(&root.join("mods/deep/b.jar")),
            sha(&b.join("mods/deep/b.jar"))
        );
        assert!(!root.join("mods/link.jar").exists());
    }

    #[test]
    fn an_export_never_overwrites_and_refuses_other_games_and_missing_versions() {
        let tmp = tempfile::tempdir().expect("tmp");
        let a = tmp.path().join("a");
        put(&a, "mods/a.jar", b"x");

        let row = sandbox(vec![mod_row("mod:1", 1)]);
        let mods = vec![deploy_mod("mod:1", 1, &a)];
        let out = tmp.path().join("o");

        let first = export_mrpack(&row, &mods, &out).expect("one");
        let second = export_mrpack(&row, &mods, &out).expect("two");

        assert_ne!(first.file_name, second.file_name);

        let mut other = row.clone();
        other.app_slug = Some("skyrim".into());
        assert!(export_mrpack(&other, &mods, &out).is_err());

        let mut bare = row.clone();
        bare.game_version = None;
        assert!(export_mrpack(&bare, &mods, &out).is_err());

        let leftovers = std::fs::read_dir(&out)
            .expect("dir")
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmcpart"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn the_share_zip_carries_the_code_and_nothing_about_this_machine() {
        let tmp = tempfile::tempdir().expect("tmp");
        let row = sandbox(vec![mod_row("mod:1", 1)]);

        let report = export_share_zip(&row, tmp.path()).expect("export");

        let mut zip = zip::ZipArchive::new(
            std::fs::File::open(tmp.path().join(&report.file_name)).expect("open"),
        )
        .expect("zip");

        let mut code = String::new();
        zip.by_name("sandbox-code.txt")
            .expect("code")
            .read_to_string(&mut code)
            .expect("text");

        assert_eq!(share::import(&code).expect("import").app_id, 42);

        let mut json = String::new();
        zip.by_name("sandbox.json")
            .expect("json")
            .read_to_string(&mut json)
            .expect("text");

        assert!(!json.contains("somebody"));
        assert!(!json.contains("secret-place"));
    }
}
