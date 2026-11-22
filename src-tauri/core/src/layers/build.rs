//! Build (or reuse) the layer of one planned manifest (docs/loaders-and-versions.md,
//! "Deploy consumption", phase 4).
//!
//! A layer lives at `<data>/layers/<app>/<layer_hash>/`:
//!
//! ```text
//!   files/             what the deploy links into the game folder
//!   .tmc-layer.json    manifest key, revision, every file's size and hash
//! ```
//!
//! It is built in `<layer_hash>.partial/` by the install executor, checked
//! ([`super::checks::scan`]), given its marker and renamed into place, so a
//! layer folder without `.partial` is always complete. A failed build keeps the
//! installer's log (copied to the app's log folder) and removes the rest.
//!
//! A layer is reused when its marker is there and every file still has its
//! size; a file whose mtime is not older than the build is hashed again. A
//! layer that fails the check is rebuilt.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::download::DownloadManager;
use crate::error::{AppError, AppResult};
use crate::layers::checks::{self, LayerFile, Limits};
use crate::layers::plan::{InstallPlan, PlannedLayer};
use crate::logging::Audit;
use crate::plugins::jail::Jail;
use crate::plugins::manifest::WhenContext;
use crate::plugins::steps::Executor;
use crate::plugins::steps_install::{install_manifest, install_roots, InstallEnv, OutputSink, RuntimeExe};

pub const MARKER: &str = ".tmc-layer.json";

/// What building a layer needs from the app.
#[derive(Clone)]
pub struct BuildCtx<'a> {
    /// The app's data folder: layers go to `layers/`, runtimes to `runtimes/`.
    pub data_root: &'a Path,
    pub http: &'a reqwest::Client,
    pub audit: &'a Audit,
    pub downloads: Option<&'a DownloadManager>,
    pub limits: Limits,
    /// Where a failed installer's log is kept.
    pub log_dir: &'a Path,
    /// The installer's proxy variables. Always empty for now: the app has no
    /// proxy setting yet, and an installer that needs one fails with its log.
    pub proxy: Vec<(String, String)>,
    pub on_output: Option<OutputSink>,
}

impl BuildCtx<'_> {
    pub fn layers_root(&self, app: &str) -> PathBuf {
        self.data_root.join("layers").join(app)
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.data_root.join("layers").join(".downloads")
    }
}

/// `.tmc-layer.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerMarker {
    pub manifest_key: String,
    pub revision: u32,
    pub layer_hash: String,
    pub files: Vec<LayerFile>,
    /// Unix seconds.
    pub built_at: u64,
}

/// A layer ready to deploy.
#[derive(Debug, Clone)]
pub struct BuiltLayer {
    pub dir: PathBuf,
    /// `dir/files`, the root of the layer's pseudo-mod.
    pub files_dir: PathBuf,
    /// The layer was already there and passed its check.
    pub reused: bool,
}

/// One build at a time: two sandboxes deploying the same build must not both
/// write its `.partial`, and installers are heavy.
fn build_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Reuse the layer of `layer`, or build it.
///
/// `runtimes` are the installed runtimes by kind, with their folders.
pub async fn ensure_layer(
    ctx: &BuildCtx<'_>,
    plan: &InstallPlan,
    layer: &PlannedLayer,
    runtimes: &HashMap<String, (RuntimeExe, PathBuf)>,
) -> AppResult<BuiltLayer> {
    let dir = ctx.layers_root(&plan.app_slug).join(&layer.layer_hash);
    let built = BuiltLayer {
        files_dir: dir.join("files"),
        dir: dir.clone(),
        reused: true,
    };

    if verify(&dir, layer).is_ok() {
        return Ok(built);
    }

    let _guard = build_lock().lock().await;

    // Another deploy may have built it while this one waited.
    match verify(&dir, layer) {
        Ok(()) => return Ok(built),
        Err(why) if dir.exists() => {
            crate::audit!(
                ctx.audit,
                Warn,
                Install,
                "layer.rebuild",
                format!("Rebuilding {} ({}): {why}", layer.manifest_key, short(&layer.layer_hash))
            );
            std::fs::remove_dir_all(&dir)?;
        }
        Err(_) => {}
    }

    build(ctx, plan, layer, runtimes, &dir).await?;

    Ok(BuiltLayer { reused: false, ..built })
}

async fn build(
    ctx: &BuildCtx<'_>,
    plan: &InstallPlan,
    layer: &PlannedLayer,
    runtimes: &HashMap<String, (RuntimeExe, PathBuf)>,
    dir: &Path,
) -> AppResult<()> {
    let partial = partial_of(dir);
    if partial.exists() {
        std::fs::remove_dir_all(&partial)?;
    }
    std::fs::create_dir_all(partial.join("files"))?;
    let downloads = ctx.downloads_dir();
    std::fs::create_dir_all(&downloads)?;

    let result = run(ctx, plan, layer, runtimes, &partial, &downloads).await;

    if let Err(e) = result {
        let log = partial.join("work").join("installer.log");
        let kept = if log.is_file() {
            let name = format!(
                "installer-{}-{}.log",
                layer.manifest_key.replace('/', "-"),
                short(&layer.layer_hash)
            );
            let to = ctx.log_dir.join(name);
            std::fs::create_dir_all(ctx.log_dir).ok();
            std::fs::copy(&log, &to).ok().map(|_| to)
        } else {
            None
        };
        let _ = std::fs::remove_dir_all(&partial);
        let msg = match kept {
            Some(to) => format!("{} The installer's log is {}.", e.detail(), to.display()),
            None => e.detail(),
        };
        crate::audit!(
            ctx.audit,
            Warn,
            Install,
            "layer.build.failed",
            format!("Building {} failed: {msg}", layer.manifest_key)
        );
        return Err(AppError::invalid(msg));
    }

    let finish = || -> AppResult<()> {
        let work = partial.join("work");
        if work.exists() {
            std::fs::remove_dir_all(&work)?;
        }
        let files = checks::scan(&partial.join("files"), ctx.limits)?;
        let marker = LayerMarker {
            manifest_key: layer.manifest_key.clone(),
            revision: layer.revision,
            layer_hash: layer.layer_hash.clone(),
            files,
            built_at: now_secs(),
        };
        let text = serde_json::to_vec_pretty(&marker)
            .map_err(|e| AppError::internal(format!("Could not write the layer marker: {e}")))?;
        std::fs::write(partial.join(MARKER), text)?;
        std::fs::rename(&partial, dir)?;
        Ok(())
    };

    if let Err(e) = finish() {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(AppError::invalid(format!(
            "The {} install was refused: {}",
            layer.title,
            e.detail()
        )));
    }

    crate::audit!(
        ctx.audit,
        Info,
        Install,
        "layer.built",
        format!("Built {} ({})", layer.manifest_key, short(&layer.layer_hash))
    );
    Ok(())
}

async fn run(
    ctx: &BuildCtx<'_>,
    plan: &InstallPlan,
    layer: &PlannedLayer,
    runtimes: &HashMap<String, (RuntimeExe, PathBuf)>,
    partial: &Path,
    downloads: &Path,
) -> AppResult<()> {
    let manifest = install_manifest(&layer.manifest_key, &layer.title, &layer.download_hosts);
    let runtime_root = runtimes.values().next().map(|(_, root)| root.as_path());
    let roots = install_roots(partial, downloads, runtime_root);
    let jail = Jail::build(&manifest, &roots)?;
    let exec = Executor {
        manifest: &manifest,
        jail: &jail,
        http: ctx.http,
        audit: ctx.audit,
        downloads: ctx.downloads,
    };
    let env = InstallEnv {
        values: layer.values(),
        when: WhenContext {
            side: plan.side.clone(),
            platform: plan.platform.clone(),
            consents: plan.consent_ids().into_iter().collect::<BTreeSet<_>>(),
        },
        artifacts: layer.artifacts.clone(),
        runtimes: runtimes.iter().map(|(k, (exe, _))| (k.clone(), exe.clone())).collect(),
        proxy: ctx.proxy.clone(),
        on_output: ctx.on_output.clone(),
    };

    let report = exec.run_install(&layer.install_steps, &env).await;
    if report.run.ok {
        return Ok(());
    }

    let at = report.run.failed_at.map(|i| format!(" at step {}", i + 1)).unwrap_or_default();
    Err(AppError::invalid(format!(
        "Installing {} failed{at}: {}",
        layer.title,
        report.run.error.unwrap_or_else(|| "unknown error".into())
    )))
}

/// `Ok` when `dir` holds a complete, untouched build of `layer`.
pub fn verify(dir: &Path, layer: &PlannedLayer) -> Result<(), String> {
    let text = std::fs::read(dir.join(MARKER)).map_err(|_| "no marker".to_string())?;
    let marker: LayerMarker =
        serde_json::from_slice(&text).map_err(|e| format!("the marker is unreadable: {e}"))?;
    if marker.layer_hash != layer.layer_hash || marker.manifest_key != layer.manifest_key {
        return Err("the marker is for another layer".into());
    }

    let files = dir.join("files");
    let mut seen = 0usize;
    count_files(&files, &mut seen)?;
    if seen != marker.files.len() {
        return Err(format!("{seen} files where the marker lists {}", marker.files.len()));
    }

    for f in &marker.files {
        let path = files.join(&f.path);
        let meta = std::fs::symlink_metadata(&path).map_err(|_| format!("{} is missing", f.path))?;
        if !meta.file_type().is_file() {
            return Err(format!("{} is not a regular file", f.path));
        }
        if meta.len() != f.size {
            return Err(format!("{} changed size", f.path));
        }
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(u64::MAX);
        if mtime >= marker.built_at {
            let sha = checks::sha256_file(&path).map_err(|e| e.detail())?;
            if sha != f.sha256 {
                return Err(format!("{} changed", f.path));
            }
        }
    }

    Ok(())
}

fn count_files(dir: &Path, seen: &mut usize) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            count_files(&entry.path(), seen)?;
        } else if kind.is_file() {
            *seen += 1;
        } else {
            return Err(format!("{} is not a regular file", entry.path().display()));
        }
    }
    Ok(())
}

fn partial_of(dir: &Path) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    dir.with_file_name(name)
}

pub(crate) fn short(hash: &str) -> &str {
    &hash[..hash.len().min(12)]
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
