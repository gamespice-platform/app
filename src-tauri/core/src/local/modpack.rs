//! Modpack archives: Modrinth `.mrpack` and CurseForge manifest zips.
//!
//! A pack is a zip holding a manifest and an `overrides/` folder. Importing one
//! goes through the same door as any other dropped archive
//! ([`super::store::prepare`]); this file only knows how to read the manifest
//! and how to bring its listed files in safely.
//!
//! WHAT THE PACK IS TRUSTED FOR
//! ----------------------------
//! Nothing it does not earn. The manifest is a stranger's file, so:
//!
//! * every path in it goes through [`join_relative`] before anything is written;
//! * `overrides/` is unpacked by the shared extractor (zip-slip checks, byte and
//!   entry caps) and copied by the importer's own tree copy;
//! * a listed download needs `https`, a host on [`ALLOWED_HOSTS`] (the hosts the
//!   Modrinth format itself permits) and a SHA-512 the pack declares. The bytes
//!   arrive through the download queue into a staging folder that is NOT the
//!   mod's deploy root, and are moved into place only after [`finalize`] has
//!   hashed them. A mismatch deletes the file.
//! * no step runs a program and the webview never names a path: commands take
//!   a mod id.
//!
//! CurseForge manifests name files by project and file id only; resolving
//! those needs CurseForge's keyed API, which this app does not call. Their
//! `overrides/` are imported and the mod list is reported as unresolved.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

use crate::download::DownloadRequest;
use crate::error::{AppError, AppResult};
use crate::plugins::jail::join_relative;

use super::store::{local_root, sanitise_rel};

/// Hosts a Modrinth pack may fetch from: the list the `.mrpack` format fixes.
pub const ALLOWED_HOSTS: &[&str] = &[
    "cdn.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
];

/// A manifest larger than this is not a manifest.
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;

/// Files one pack may list.
pub const MAX_PACK_FILES: usize = 20_000;

const PACKS_DIR: &str = ".packs";
const PLAN_FILE: &str = "plan.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PackKind {
    Mrpack,
    Curseforge,
}

/// One file a pack asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackFile {
    /// Relative to the game folder, already cleaned.
    pub path: String,
    pub urls: Vec<String>,
    /// Lower-case hex, 128 characters.
    pub sha512: String,
    pub size: Option<u64>,
}

/// What a pack says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackManifest {
    pub kind: PackKind,
    pub name: String,
    pub version: Option<String>,
    pub game_version: Option<String>,
    pub loader: Option<String>,
    /// Folders in the zip whose contents are copied over the game folder.
    pub override_dirs: Vec<String>,
    /// Files to download. Empty for CurseForge.
    pub files: Vec<PackFile>,
    /// Entries left out because the pack marks them client-unsupported.
    pub skipped_server_only: usize,
    /// `project/file` ids a CurseForge pack names that this app cannot fetch.
    pub unresolved: Vec<String>,
}

/// Read a zip's pack manifest, if it is a pack.
///
/// `Ok(None)` for anything that is not a pack, including a zip that merely has
/// a `manifest.json`. A pack whose manifest is present but unreadable is an
/// error, not a plain archive: unpacking it would scatter its folders over the
/// game.
pub fn read(archive: &Path) -> AppResult<Option<PackManifest>> {
    let lower = archive
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if !(lower.ends_with(".zip") || lower.ends_with(".mrpack")) {
        return Ok(None);
    }

    let Ok(file) = std::fs::File::open(archive) else {
        return Ok(None);
    };

    let Ok(mut zip) = zip::ZipArchive::new(file) else {
        return Ok(None);
    };

    if let Some(raw) = entry_text(&mut zip, "modrinth.index.json")? {
        return parse_mrpack(&raw).map(Some);
    }

    if let Some(raw) = entry_text(&mut zip, "manifest.json")? {
        let is_pack = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|v| {
                v.get("manifestType")
                    .and_then(|t| t.as_str())
                    .map(str::to_owned)
            })
            .is_some_and(|t| t == "minecraftModpack");

        if is_pack {
            return parse_curseforge(&raw).map(Some);
        }
    }

    Ok(None)
}

fn entry_text(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> AppResult<Option<String>> {
    let Ok(entry) = zip.by_name(name) else {
        return Ok(None);
    };

    let mut raw = Vec::new();

    entry
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|e| AppError::invalid(format!("could not read {name}: {e}")))?;

    if raw.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(AppError::invalid(format!("{name} is too large.")));
    }

    String::from_utf8(raw)
        .map(Some)
        .map_err(|_| AppError::invalid(format!("{name} is not text.")))
}

#[derive(Deserialize)]
struct MrIndex {
    #[serde(rename = "formatVersion")]
    format_version: Option<u32>,
    game: Option<String>,
    #[serde(rename = "versionId")]
    version_id: Option<String>,
    name: Option<String>,
    #[serde(default)]
    files: Vec<MrFile>,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct MrFile {
    path: String,
    #[serde(default)]
    hashes: BTreeMap<String, String>,
    #[serde(default)]
    downloads: Vec<String>,
    #[serde(rename = "fileSize")]
    file_size: Option<u64>,
    env: Option<MrEnv>,
}

#[derive(Deserialize)]
struct MrEnv {
    client: Option<String>,
}

/// Parse a Modrinth `modrinth.index.json`.
pub fn parse_mrpack(raw: &str) -> AppResult<PackManifest> {
    let index: MrIndex = serde_json::from_str(raw)
        .map_err(|e| AppError::invalid(format!("modrinth.index.json is not valid: {e}")))?;

    if index.format_version != Some(1) {
        return Err(AppError::invalid(
            "That modpack uses a format version this app does not read.",
        ));
    }

    if index.game.as_deref().is_some_and(|g| g != "minecraft") {
        return Err(AppError::invalid("That modpack is not for Minecraft."));
    }

    if index.files.len() > MAX_PACK_FILES {
        return Err(AppError::invalid("That modpack lists too many files."));
    }

    let mut files = Vec::new();
    let mut skipped = 0usize;
    let mut seen = std::collections::BTreeSet::new();

    for file in index.files {
        // This app installs on a player's machine. A file the pack says the
        // client does not use is not fetched.
        if file
            .env
            .as_ref()
            .and_then(|e| e.client.as_deref())
            .is_some_and(|c| c == "unsupported")
        {
            skipped += 1;

            continue;
        }

        // `sanitise_rel` forgives a leading slash; a pack that has one is
        // asking for an absolute path, and is refused rather than corrected.
        if file.path.starts_with(['/', '\\']) {
            return Err(AppError::invalid(format!(
                "{:?} is not a safe path.",
                file.path
            )));
        }

        let path = sanitise_rel(&file.path)
            .map_err(|_| AppError::invalid(format!("{:?} is not a safe path.", file.path)))?;

        if path.is_empty() || !seen.insert(path.to_ascii_lowercase()) {
            return Err(AppError::invalid(format!(
                "{:?} is empty or listed twice.",
                file.path
            )));
        }

        let sha512 = file
            .hashes
            .get("sha512")
            .map(|h| h.to_ascii_lowercase())
            .filter(|h| h.len() == 128 && h.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| {
                AppError::invalid(format!(
                    "{path} has no usable sha512, so it cannot be verified."
                ))
            })?;

        if file.downloads.is_empty() {
            return Err(AppError::invalid(format!("{path} lists no download.")));
        }

        for url in &file.downloads {
            check_url(url)?;
        }

        files.push(PackFile {
            path,
            urls: file.downloads,
            sha512,
            size: file.file_size,
        });
    }

    let loader = ["fabric-loader", "quilt-loader", "neoforge", "forge"]
        .iter()
        .find(|k| index.dependencies.contains_key(**k))
        .map(|k| (*k).to_string());

    Ok(PackManifest {
        kind: PackKind::Mrpack,
        name: clean_name(index.name.as_deref()),
        version: index.version_id.filter(|v| !v.trim().is_empty()),
        game_version: index.dependencies.get("minecraft").cloned(),
        loader,
        override_dirs: vec!["overrides".into(), "client-overrides".into()],
        files,
        skipped_server_only: skipped,
        unresolved: Vec::new(),
    })
}

fn check_url(raw: &str) -> AppResult<()> {
    let url = url::Url::parse(raw)
        .map_err(|_| AppError::invalid(format!("{raw:?} is not a valid download URL.")))?;

    let host = url.host_str().unwrap_or_default();

    if url.scheme() != "https" || url.port().is_some() || !ALLOWED_HOSTS.contains(&host) {
        return Err(AppError::invalid(format!(
            "The pack downloads from {host:?}, which is not a host modpacks may use."
        )));
    }

    Ok(())
}

fn clean_name(name: Option<&str>) -> String {
    name.map(|n| n.trim().chars().take(200).collect::<String>())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Modpack".into())
}

#[derive(Deserialize)]
struct CfManifest {
    name: Option<String>,
    version: Option<String>,
    minecraft: Option<CfMinecraft>,
    #[serde(default)]
    files: Vec<CfFile>,
    overrides: Option<String>,
}

#[derive(Deserialize)]
struct CfMinecraft {
    version: Option<String>,
    #[serde(default, rename = "modLoaders")]
    mod_loaders: Vec<CfLoader>,
}

#[derive(Deserialize)]
struct CfLoader {
    id: Option<String>,
    #[serde(default)]
    primary: bool,
}

#[derive(Deserialize)]
struct CfFile {
    #[serde(rename = "projectID")]
    project_id: u64,
    #[serde(rename = "fileID")]
    file_id: u64,
}

/// Parse a CurseForge `manifest.json`.
pub fn parse_curseforge(raw: &str) -> AppResult<PackManifest> {
    let manifest: CfManifest = serde_json::from_str(raw)
        .map_err(|e| AppError::invalid(format!("manifest.json is not valid: {e}")))?;

    if manifest.files.len() > MAX_PACK_FILES {
        return Err(AppError::invalid("That modpack lists too many files."));
    }

    // The folder name comes from the manifest, so it is one path component or
    // nothing.
    let dir = manifest.overrides.as_deref().unwrap_or("overrides");

    if dir.is_empty() || dir.contains(['/', '\\']) || dir == "." || dir == ".." {
        return Err(AppError::invalid(
            "That modpack names an overrides folder this app will not use.",
        ));
    }

    let minecraft = manifest.minecraft;

    let loader = minecraft.as_ref().and_then(|m| {
        m.mod_loaders
            .iter()
            .find(|l| l.primary)
            .or(m.mod_loaders.first())
            .and_then(|l| l.id.clone())
    });

    Ok(PackManifest {
        kind: PackKind::Curseforge,
        name: clean_name(manifest.name.as_deref()),
        version: manifest.version.filter(|v| !v.trim().is_empty()),
        game_version: minecraft.and_then(|m| m.version),
        loader,
        override_dirs: vec![dir.to_string()],
        files: Vec::new(),
        skipped_server_only: 0,
        unresolved: manifest
            .files
            .iter()
            .map(|f| format!("{}/{}", f.project_id, f.file_id))
            .collect(),
    })
}

/// Unpack a pack's override folders into `dest`.
///
/// Through the shared extractor into a scratch folder first, then `copy` (the
/// importer's own tree copy), so the zip-slip checks and the byte and entry
/// caps are the ones every other archive gets, and a symlink is never copied.
pub fn unpack_overrides(
    archive: &Path,
    dest: &Path,
    pack: &PackManifest,
    copy: impl Fn(&Path, &Path) -> AppResult<(usize, u64)>,
) -> AppResult<(usize, u64)> {
    let scratch = dest.with_file_name("pack-scratch");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)?;

    let result = (|| {
        crate::plugins::steps::extract_archive(archive, &scratch, &Default::default())?;

        let mut files = 0usize;
        let mut bytes = 0u64;

        // Later folders win, as in the format: client-overrides over overrides.
        for dir in &pack.override_dirs {
            let from = join_relative(&scratch, dir)?;

            if from.is_dir() {
                let (f, b) = copy(&from, dest)?;
                files += f;
                bytes += b;
            }
        }

        Ok((files, bytes))
    })();

    let _ = std::fs::remove_dir_all(&scratch);

    result
}

fn pack_dir(store: &Path, id: i64) -> PathBuf {
    store.join(PACKS_DIR).join(id.to_string())
}

/// Remember what is still to be fetched for local mod `id`.
pub fn save_plan(store: &Path, id: i64, pack: &PackManifest) -> AppResult<()> {
    if pack.files.is_empty() {
        return Ok(());
    }

    let dir = pack_dir(store, id);
    std::fs::create_dir_all(&dir)?;

    let json = serde_json::to_vec(&pack.files)
        .map_err(|e| AppError::internal(format!("could not save the pack plan: {e}")))?;

    std::fs::write(dir.join(PLAN_FILE), json)?;

    Ok(())
}

fn load_plan(store: &Path, id: i64) -> Vec<PackFile> {
    std::fs::read(pack_dir(store, id).join(PLAN_FILE))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default()
}

fn staged(store: &Path, id: i64, index: usize) -> PathBuf {
    pack_dir(store, id).join(format!("{index}.bin"))
}

/// Queue requests for every file in the saved plan.
///
/// Destinations sit in the pack's staging folder, never the mod's root, so a
/// half-fetched or unverified file can not be deployed.
pub fn download_requests(store: &Path, id: i64, mod_name: &str) -> Vec<DownloadRequest> {
    load_plan(store, id)
        .into_iter()
        .enumerate()
        .map(|(index, file)| DownloadRequest {
            id: format!("pack{id}:{index}"),
            url: file.urls.first().cloned().unwrap_or_default(),
            dest: staged(store, id, index),
            label: format!("{mod_name}: {}", file.path),
            sha256: None,
            size_hint: file.size,
            priority: 0,
            limit_bps: None,
            meta: BTreeMap::from([("pack".to_string(), id.to_string())]),
        })
        .collect()
}

/// What [`finalize`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finalized {
    pub placed: usize,
    /// Not downloaded yet.
    pub pending: usize,
    /// Downloaded but refused, by path. The staged copy is deleted.
    pub failed: Vec<String>,
}

/// Verify what has arrived and move it into the mod. Safe to call repeatedly.
pub fn finalize(store: &Path, id: i64, rel_path: &str) -> AppResult<Finalized> {
    let plan = load_plan(store, id);
    let mut out = Finalized::default();

    let base = {
        let root = local_root(store, id);

        if rel_path.is_empty() {
            root
        } else {
            join_relative(&root, rel_path)?
        }
    };

    for (index, file) in plan.iter().enumerate() {
        let from = staged(store, id, index);

        if !from.is_file() {
            // Placed on an earlier call, or not here yet.
            if join_relative(&base, &file.path).is_ok_and(|p| p.is_file()) {
                out.placed += 1;
            } else {
                out.pending += 1;
            }

            continue;
        }

        let verified = sha512_of(&from).is_some_and(|d| d == file.sha512)
            && file
                .size
                .is_none_or(|s| std::fs::metadata(&from).is_ok_and(|m| m.len() == s));

        if !verified {
            let _ = std::fs::remove_file(&from);
            out.failed.push(file.path.clone());

            continue;
        }

        let to = join_relative(&base, &file.path)?;

        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }

        if std::fs::rename(&from, &to).is_err() {
            std::fs::copy(&from, &to)?;
            let _ = std::fs::remove_file(&from);
        }

        out.placed += 1;
    }

    if out.pending == 0 && out.failed.is_empty() {
        let _ = std::fs::remove_dir_all(pack_dir(store, id));
    }

    Ok(out)
}

/// Forget a mod's pending plan and staged bytes.
pub fn discard(store: &Path, id: i64) {
    let _ = std::fs::remove_dir_all(pack_dir(store, id));
}

fn sha512_of(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha512::new();
    let mut buf = [0u8; 64 * 1024];

    loop {
        let n = file.read(&mut buf).ok()?;

        if n == 0 {
            break;
        }

        hasher.update(&buf[..n]);
    }

    Some(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: &str) -> String {
        byte.repeat(64)
    }

    fn index(files: &str) -> String {
        format!(
            r#"{{"formatVersion":1,"game":"minecraft","versionId":"1.2","name":"Pack","files":[{files}],
               "dependencies":{{"minecraft":"1.20.1","fabric-loader":"0.15"}}}}"#
        )
    }

    fn file(path: &str, url: &str, extra: &str) -> String {
        format!(
            r#"{{"path":"{path}","hashes":{{"sha1":"x","sha512":"{}"}},"downloads":["{url}"],"fileSize":3 {extra}}}"#,
            hash("ab")
        )
    }

    fn write_zip(path: &Path, entries: &[(&str, &str)]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).expect("create"));
        let opts: zip::write::FileOptions<'_, '_, ()> = zip::write::FileOptions::default();

        for (name, body) in entries {
            zip.start_file(*name, opts).expect("entry");
            std::io::Write::write_all(&mut zip, body.as_bytes()).expect("body");
        }

        zip.finish().expect("finish");
    }

    #[test]
    fn an_mrpack_index_is_read() {
        let raw = index(&file(
            "mods/a.jar",
            "https://cdn.modrinth.com/data/a.jar",
            "",
        ));
        let pack = parse_mrpack(&raw).expect("parse");

        assert_eq!(pack.name, "Pack");
        assert_eq!(pack.game_version.as_deref(), Some("1.20.1"));
        assert_eq!(pack.loader.as_deref(), Some("fabric-loader"));
        assert_eq!(pack.files.len(), 1);
        assert_eq!(pack.files[0].path, "mods/a.jar");
    }

    #[test]
    fn client_unsupported_files_are_skipped() {
        let raw = index(&file(
            "mods/s.jar",
            "https://cdn.modrinth.com/s.jar",
            r#","env":{"client":"unsupported","server":"required"}"#,
        ));
        let pack = parse_mrpack(&raw).expect("parse");

        assert!(pack.files.is_empty());
        assert_eq!(pack.skipped_server_only, 1);
    }

    #[test]
    fn unsafe_paths_hosts_and_hashes_are_refused() {
        for path in ["../x.jar", "/etc/x", "C:/x.jar"] {
            let raw = index(&file(path, "https://cdn.modrinth.com/x", ""));
            assert!(parse_mrpack(&raw).is_err(), "{path}");
        }

        for url in [
            "http://cdn.modrinth.com/x",
            "https://evil.example/x",
            "https://cdn.modrinth.com:8443/x",
            "https://cdn.modrinth.com.evil.example/x",
        ] {
            let raw = index(&file("mods/x.jar", url, ""));
            assert!(parse_mrpack(&raw).is_err(), "{url}");
        }

        let no_hash = index(
            r#"{"path":"mods/x.jar","hashes":{"sha1":"x"},"downloads":["https://cdn.modrinth.com/x"]}"#,
        );
        assert!(parse_mrpack(&no_hash).is_err());

        let twice = format!(
            "{},{}",
            file("mods/x.jar", "https://cdn.modrinth.com/x", ""),
            file("MODS/X.jar", "https://cdn.modrinth.com/y", "")
        );
        assert!(parse_mrpack(&index(&twice)).is_err());
    }

    #[test]
    fn a_curseforge_manifest_reports_unresolved_files_and_guards_the_overrides_name() {
        let raw = r#"{"manifestType":"minecraftModpack","name":"CF","version":"3",
            "minecraft":{"version":"1.20.1","modLoaders":[{"id":"forge-47.1.0","primary":true}]},
            "files":[{"projectID":10,"fileID":20,"required":true}],"overrides":"overrides"}"#;
        let pack = parse_curseforge(raw).expect("parse");

        assert_eq!(pack.unresolved, vec!["10/20"]);
        assert_eq!(pack.loader.as_deref(), Some("forge-47.1.0"));

        let bad = raw.replace(r#""overrides":"overrides""#, r#""overrides":"../x""#);
        assert!(parse_curseforge(&bad).is_err());
    }

    #[test]
    fn read_tells_packs_from_plain_zips() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mr = tmp.path().join("p.mrpack");
        write_zip(
            &mr,
            &[(
                "modrinth.index.json",
                &index(&file("mods/a.jar", "https://cdn.modrinth.com/a", "")),
            )],
        );
        assert_eq!(
            read(&mr).expect("read").expect("pack").kind,
            PackKind::Mrpack
        );

        let plain = tmp.path().join("m.zip");
        write_zip(&plain, &[("manifest.json", r#"{"name":"not a pack"}"#)]);
        assert!(read(&plain).expect("read").is_none());

        let broken = tmp.path().join("b.mrpack");
        write_zip(&broken, &[("modrinth.index.json", "{nope")]);
        assert!(read(&broken).is_err());
    }

    #[test]
    fn finalize_places_verified_files_and_deletes_mismatches() {
        let store = tempfile::tempdir().expect("tmp");
        let id = 7;
        let good = b"abc";

        let pack = PackManifest {
            kind: PackKind::Mrpack,
            name: "P".into(),
            version: None,
            game_version: None,
            loader: None,
            override_dirs: vec![],
            files: vec![
                PackFile {
                    path: "mods/good.jar".into(),
                    urls: vec!["https://cdn.modrinth.com/g".into()],
                    sha512: hex::encode(Sha512::digest(good)),
                    size: Some(3),
                },
                PackFile {
                    path: "mods/bad.jar".into(),
                    urls: vec!["https://cdn.modrinth.com/b".into()],
                    sha512: hash("cd"),
                    size: None,
                },
                PackFile {
                    path: "mods/later.jar".into(),
                    urls: vec!["https://cdn.modrinth.com/l".into()],
                    sha512: hash("ef"),
                    size: None,
                },
            ],
            skipped_server_only: 0,
            unresolved: vec![],
        };

        save_plan(store.path(), id, &pack).expect("plan");
        std::fs::create_dir_all(local_root(store.path(), id)).expect("root");

        let requests = download_requests(store.path(), id, "P");
        assert_eq!(requests.len(), 3);
        assert!(requests[0].dest.starts_with(store.path().join(".packs")));

        std::fs::write(&requests[0].dest, good).expect("stage");
        std::fs::write(&requests[1].dest, b"tampered").expect("stage");

        let done = finalize(store.path(), id, "").expect("finalize");

        assert_eq!(done.placed, 1);
        assert_eq!(done.pending, 1);
        assert_eq!(done.failed, vec!["mods/bad.jar"]);
        assert!(local_root(store.path(), id).join("mods/good.jar").is_file());
        assert!(!local_root(store.path(), id).join("mods/bad.jar").exists());
        assert!(!requests[1].dest.exists());

        // Running it again is harmless and still counts the placed file.
        let again = finalize(store.path(), id, "").expect("again");
        assert_eq!(again.placed, 1);
    }

    #[test]
    fn overrides_unpack_with_client_over_common_and_nothing_else() {
        let tmp = tempfile::tempdir().expect("tmp");
        let zip = tmp.path().join("p.mrpack");
        write_zip(
            &zip,
            &[
                ("modrinth.index.json", &index("")),
                ("overrides/config/a.txt", "common"),
                ("overrides/config/b.txt", "b"),
                ("client-overrides/config/a.txt", "client"),
                ("stray.txt", "x"),
            ],
        );

        let pack = read(&zip).expect("read").expect("pack");
        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).expect("dest");

        let copy = |from: &Path, to: &Path| super::super::store::copy_tree(from, to);
        let (files, _) = unpack_overrides(&zip, &dest, &pack, copy).expect("unpack");

        assert_eq!(files, 3);
        assert_eq!(
            std::fs::read_to_string(dest.join("config/a.txt")).expect("a"),
            "client"
        );
        assert!(dest.join("config/b.txt").is_file());
        assert!(!dest.join("stray.txt").exists());
        assert!(!tmp.path().join("pack-scratch").exists());
    }
}
