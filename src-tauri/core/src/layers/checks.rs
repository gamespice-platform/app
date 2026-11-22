//! What a built layer may contain (docs/loader-manifests.md, "Layer checks").
//!
//! A layer's `files/` folder is what deployment links into the game folder,
//! and an installer wrote it. Before a layer is published every entry is
//! checked:
//!
//! - regular files and folders only: a symlink is refused (and named when it
//!   points outside the layer), as are sockets, fifos and devices;
//! - no file with more than one hard link, which would let a later write
//!   through another name change a file after it was hashed;
//! - no component that Windows cannot hold: a device name (`CON`, `nul.txt`)
//!   or a trailing dot or space;
//! - no two paths that differ only by case, which collide on Windows and macOS;
//! - at most [`Limits`] bytes and files (settings `layerMaxBytes`,
//!   `layerMaxFiles`; `0` is unlimited).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};
use crate::settings::AppSettings;

/// The size ceilings of one layer. `0` is unlimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_bytes: u64,
    pub max_files: u64,
}

impl Limits {
    pub const UNLIMITED: Limits = Limits {
        max_bytes: 0,
        max_files: 0,
    };

    pub fn from_settings(settings: &AppSettings) -> Self {
        Self {
            max_bytes: settings.layer_max_bytes,
            max_files: settings.layer_max_files,
        }
    }
}

/// One file of a layer, as its marker records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerFile {
    /// Relative to `files/`, `/`-separated.
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// Check every entry under `root` and hash every file, sorted by path.
pub fn scan(root: &Path, limits: Limits) -> AppResult<Vec<LayerFile>> {
    let mut out = Vec::new();
    let mut lower: HashMap<String, String> = HashMap::new();
    let mut total: u64 = 0;

    walk(
        root,
        root,
        String::new(),
        limits,
        &mut out,
        &mut lower,
        &mut total,
    )?;

    out.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(out)
}

fn walk(
    root: &Path,
    dir: &Path,
    prefix: String,
    limits: Limits,
    out: &mut Vec<LayerFile>,
    lower: &mut HashMap<String, String>,
    total: &mut u64,
) -> AppResult<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            refused(format!(
                "{} has a name that is not valid Unicode",
                entry.path().display()
            ))
        })?;
        let rel = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };

        check_component(&rel, name)?;

        if let Some(other) = lower.insert(rel.to_lowercase(), rel.clone()) {
            return Err(refused(format!(
                "{rel} and {other} differ only by case, which collide on Windows and macOS"
            )));
        }

        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)?;
        let kind = meta.file_type();

        if kind.is_symlink() {
            return Err(refused(symlink_message(root, &path, &rel)));
        }

        if kind.is_dir() {
            walk(root, &path, rel, limits, out, lower, total)?;
            continue;
        }

        if !kind.is_file() {
            return Err(refused(format!("{rel} is not a regular file")));
        }

        if links(&meta) > 1 {
            return Err(refused(format!(
                "{rel} has {} hard links; a layer file must have exactly one",
                links(&meta)
            )));
        }

        *total = total.saturating_add(meta.len());

        if limits.max_bytes != 0 && *total > limits.max_bytes {
            return Err(refused(format!(
                "the layer is larger than the {} byte limit (Settings, layer size limit; 0 is unlimited)",
                limits.max_bytes
            )));
        }

        if limits.max_files != 0 && out.len() as u64 >= limits.max_files {
            return Err(refused(format!(
                "the layer has more than {} files (Settings, layer file limit; 0 is unlimited)",
                limits.max_files
            )));
        }

        out.push(LayerFile {
            path: rel,
            size: meta.len(),
            sha256: sha256_file(&path)?,
        });
    }

    Ok(())
}

fn refused(detail: String) -> AppError {
    AppError::jail(format!("The built layer was refused: {detail}."))
}

/// A device name or a trailing dot or space.
fn check_component(rel: &str, name: &str) -> AppResult<()> {
    if crate::canon::is_device_name(name) {
        return Err(refused(format!("{rel} is a Windows device name")));
    }

    if name.ends_with('.') || name.ends_with(' ') {
        return Err(refused(format!("{rel} ends with a dot or a space")));
    }

    Ok(())
}

fn symlink_message(root: &Path, link: &Path, rel: &str) -> String {
    let target = std::fs::read_link(link).unwrap_or_default();
    let resolved: PathBuf = if target.is_absolute() {
        target.clone()
    } else {
        link.parent().unwrap_or(root).join(&target)
    };

    if escapes(root, &resolved) {
        format!(
            "{rel} is a symlink to {}, outside the layer",
            target.display()
        )
    } else {
        format!("{rel} is a symlink; layers hold regular files only")
    }
}

/// Lexically, without touching the disk: whether `path` leaves `root`.
fn escapes(root: &Path, path: &Path) -> bool {
    use std::path::Component;

    let mut parts: Vec<Component<'_>> = Vec::new();

    for c in path.components() {
        match c {
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return true;
                }
            }
            Component::CurDir => {}
            other => parts.push(other),
        }
    }

    let normal: PathBuf = parts.iter().collect();

    !normal.starts_with(root)
}

#[cfg(unix)]
fn links(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.nlink()
}

#[cfg(not(unix))]
fn links(_meta: &std::fs::Metadata) -> u64 {
    // Windows reports the link count only through a handle query; a hard link
    // made by an installer there is caught by the reuse check instead.
    1
}

/// Streaming sha256 of a file, lower-case hex.
pub fn sha256_file(path: &Path) -> AppResult<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];

    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hex(&hasher.finalize()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("files/versions")).unwrap();
        std::fs::write(tmp.path().join("files/versions/a.json"), b"{}").unwrap();
        tmp
    }

    #[test]
    fn a_plain_layer_is_hashed_in_path_order() {
        let tmp = layer();
        std::fs::write(tmp.path().join("files/b.txt"), b"b").unwrap();
        let files = scan(&tmp.path().join("files"), Limits::UNLIMITED).unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["b.txt", "versions/a.json"]);
        assert_eq!(files[1].size, 2);
        assert_eq!(files[1].sha256.len(), 64);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_refused_and_an_escaping_one_is_named() {
        let tmp = layer();
        std::os::unix::fs::symlink("/etc/passwd", tmp.path().join("files/evil")).unwrap();
        let e = scan(&tmp.path().join("files"), Limits::UNLIMITED).unwrap_err();
        assert!(e.detail().contains("/etc/passwd"), "{}", e.detail());
        assert!(e.detail().contains("outside the layer"));

        std::fs::remove_file(tmp.path().join("files/evil")).unwrap();
        std::os::unix::fs::symlink("versions/a.json", tmp.path().join("files/inner")).unwrap();
        let e = scan(&tmp.path().join("files"), Limits::UNLIMITED).unwrap_err();
        assert!(e.detail().contains("is a symlink"), "{}", e.detail());
    }

    #[cfg(unix)]
    #[test]
    fn hard_links_are_refused() {
        let tmp = layer();
        std::fs::hard_link(
            tmp.path().join("files/versions/a.json"),
            tmp.path().join("files/b.json"),
        )
        .unwrap();
        let e = scan(&tmp.path().join("files"), Limits::UNLIMITED).unwrap_err();
        assert!(e.detail().contains("hard links"), "{}", e.detail());
    }

    #[test]
    fn windows_hostile_names_and_case_collisions_are_refused() {
        for bad in ["nul.txt", "CON", "trailing.", "space "] {
            let tmp = layer();
            std::fs::write(tmp.path().join("files").join(bad), b"x").unwrap();
            assert!(
                scan(&tmp.path().join("files"), Limits::UNLIMITED).is_err(),
                "{bad}"
            );
        }

        let tmp = layer();
        std::fs::write(tmp.path().join("files/Readme"), b"x").unwrap();
        std::fs::write(tmp.path().join("files/README"), b"x").unwrap();
        let e = scan(&tmp.path().join("files"), Limits::UNLIMITED).unwrap_err();
        assert!(e.detail().contains("differ only by case"));
    }

    #[test]
    fn the_limits_apply_and_zero_is_unlimited() {
        let tmp = layer();
        std::fs::write(tmp.path().join("files/big"), vec![0u8; 100]).unwrap();
        let root = tmp.path().join("files");
        assert!(scan(
            &root,
            Limits {
                max_bytes: 50,
                max_files: 0
            }
        )
        .is_err());
        assert!(scan(
            &root,
            Limits {
                max_bytes: 0,
                max_files: 1
            }
        )
        .is_err());
        assert!(scan(
            &root,
            Limits {
                max_bytes: 1000,
                max_files: 2
            }
        )
        .is_ok());
    }
}
