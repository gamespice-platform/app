//! A dropped folder or archive as a flat, ordered file tree: what the picker
//! shows when no metadata file says what the thing is.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Entries beyond this are not listed (the tree says so). A display bound, not
/// a limit on what installs: an install takes the whole payload.
pub const MAX_NODES: usize = 5000;

/// The biggest metadata file read from a candidate. Bounds a hostile archive.
const MAX_META_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    /// Forward slashes, relative to the root; no leading slash.
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTree {
    pub nodes: Vec<TreeNode>,
    /// File paths the picker should tick by default.
    pub suggested: Vec<String>,
    pub total_files: usize,
    pub total_bytes: u64,
    /// True when more than [`MAX_NODES`] entries existed.
    pub truncated: bool,
}

/// A folder or a zip, read the same way.
pub enum Source {
    Dir(PathBuf),
    Zip(PathBuf),
}

impl Source {
    pub fn open(path: &Path) -> AppResult<Self> {
        if path.is_dir() {
            return Ok(Self::Dir(path.to_path_buf()));
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(ext.as_str(), "zip" | "mrpack" | "jar") {
            return Ok(Self::Zip(path.to_path_buf()));
        }
        if path.is_file() {
            return Err(AppError::invalid(
                "only a folder or a .zip can be inspected; a single file installs as it is",
            ));
        }
        Err(AppError::invalid("that path does not exist"))
    }

    /// `(relative path, is_dir, size)` for every entry, sorted by path.
    pub fn entries(&self) -> AppResult<Vec<(String, bool, u64)>> {
        let mut out = Vec::new();
        match self {
            Self::Dir(root) => walk(root, root, &mut out)?,
            Self::Zip(p) => {
                let mut z = zip::ZipArchive::new(std::fs::File::open(p)?)
                    .map_err(|e| AppError::invalid(format!("Not a readable zip: {e}")))?;
                for i in 0..z.len() {
                    let e = z
                        .by_index(i)
                        .map_err(|e| AppError::invalid(format!("Bad zip entry: {e}")))?;
                    // `enclosed_name` drops absolute and `..` paths.
                    let Some(n) = e.enclosed_name() else { continue };
                    let p = n.to_string_lossy().replace('\\', "/");
                    out.push((p.trim_end_matches('/').to_string(), e.is_dir(), e.size()));
                }
            }
        }
        out.retain(|(p, _, _)| !p.is_empty());
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    /// One file's bytes, capped; `None` when absent.
    pub fn read(&self, rel: &str) -> AppResult<Option<Vec<u8>>> {
        match self {
            Self::Dir(root) => {
                let p = root.join(rel);
                let Ok(meta) = std::fs::metadata(&p) else {
                    return Ok(None);
                };
                if !meta.is_file() || meta.len() > MAX_META_BYTES {
                    return Ok(None);
                }
                Ok(Some(std::fs::read(p)?))
            }
            Self::Zip(p) => {
                let mut z = zip::ZipArchive::new(std::fs::File::open(p)?)
                    .map_err(|e| AppError::invalid(format!("Not a readable zip: {e}")))?;
                let Ok(e) = z.by_name(rel) else {
                    return Ok(None);
                };
                if e.size() > MAX_META_BYTES {
                    return Ok(None);
                }
                let mut buf = Vec::new();
                e.take(MAX_META_BYTES).read_to_end(&mut buf)?;
                Ok(Some(buf))
            }
        }
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, bool, u64)>) -> AppResult<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        // A link could point out of the dropped folder; listing it as a file
        // of size 0 is harmless, following it is not.
        if ft.is_symlink() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let size = if ft.is_file() {
            entry.metadata()?.len()
        } else {
            0
        };
        out.push((rel, ft.is_dir(), size));
        if ft.is_dir() {
            walk(root, &entry.path(), out)?;
        }
    }
    Ok(())
}

fn ext_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_doc(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        ext_of(&n).as_str(),
        "txt" | "md" | "png" | "jpg" | "jpeg" | "gif" | "url" | "html"
    ) || n.starts_with("readme")
        || n.starts_with("license")
        || n.starts_with("changelog")
}

/// Build the tree. `mod_extensions` are the game's accepted file extensions
/// (lower-case, no dot); empty means "everything but documentation".
pub fn build(source: &Source, mod_extensions: &[String]) -> AppResult<FileTree> {
    let entries = source.entries()?;
    let mut nodes = Vec::new();
    let mut suggested = Vec::new();
    let (mut files, mut bytes) = (0usize, 0u64);
    for (path, is_dir, size) in &entries {
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        if !is_dir {
            files += 1;
            bytes += size;
            let wanted = if mod_extensions.is_empty() {
                !is_doc(&name)
            } else {
                mod_extensions.iter().any(|e| *e == ext_of(&name))
            };
            if wanted {
                suggested.push(path.clone());
            }
        }
        if nodes.len() < MAX_NODES {
            nodes.push(TreeNode {
                path: path.clone(),
                name,
                is_dir: *is_dir,
                size: *size,
                depth: path.matches('/').count(),
            });
        }
    }
    Ok(FileTree {
        truncated: entries.len() > MAX_NODES,
        nodes,
        suggested,
        total_files: files,
        total_bytes: bytes,
    })
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::io::Write;
    use std::path::Path;

    /// A zip of `(name, body)` at `path`.
    pub fn write_zip(path: &Path, files: &[(&str, &str)]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let o: zip::write::FileOptions<'_, '_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (n, b) in files {
            z.start_file(*n, o).unwrap();
            z.write_all(b.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_tree_suggests_mod_files_and_skips_docs() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("plugins")).unwrap();
        std::fs::write(d.path().join("plugins/a.dll"), "xx").unwrap();
        std::fs::write(d.path().join("README.txt"), "r").unwrap();
        let t = build(&Source::open(d.path()).unwrap(), &[]).unwrap();
        assert_eq!(t.suggested, vec!["plugins/a.dll"]);
        assert_eq!((t.total_files, t.total_bytes, t.truncated), (2, 3, false));
        assert_eq!(
            t.nodes
                .iter()
                .find(|n| n.path == "plugins/a.dll")
                .unwrap()
                .depth,
            1
        );
        let t = build(&Source::open(d.path()).unwrap(), &["txt".into()]).unwrap();
        assert_eq!(t.suggested, vec!["README.txt"]);
    }

    #[test]
    fn zip_tree_and_read() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("m.zip");
        testutil::write_zip(&z, &[("a/b.cs", "code"), ("../evil.txt", "x")]);
        let s = Source::open(&z).unwrap();
        let t = build(&s, &[]).unwrap();
        assert_eq!(t.suggested, vec!["a/b.cs"], "the ../ entry is dropped");
        assert_eq!(s.read("a/b.cs").unwrap().unwrap(), b"code");
        assert!(s.read("nope").unwrap().is_none());
    }

    #[test]
    fn single_file_and_missing_path_are_refused() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("x.dll"), "1").unwrap();
        assert!(Source::open(&d.path().join("x.dll")).is_err());
        assert!(Source::open(&d.path().join("gone")).is_err());
    }
}
