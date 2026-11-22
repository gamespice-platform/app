//! The runtimes an installer runs on (docs/loaders-and-versions.md, "Deploy
//! consumption", phase 3).
//!
//! A runtime lives at `<data>/runtimes/<kind>/<major>/<platform>/<sha12>/`:
//! `root/` is the extracted archive and `.tmc-runtime.json` says what it is.
//! It is downloaded and extracted by the same executor as a layer (pinned
//! hash, host allow-list, jail), into a `.partial` folder that is renamed
//! into place once complete.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::layers::build::{now_secs, short, BuildCtx};
use crate::layers::plan::PlannedRuntime;
use crate::plugins::jail::Jail;
use crate::plugins::manifest::{parse_install_steps, WhenContext};
use crate::plugins::steps::Executor;
use crate::plugins::steps_install::{install_manifest, install_roots, InstallEnv, RuntimeExe};

pub const MARKER: &str = ".tmc-runtime.json";

/// `.tmc-runtime.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeMarker {
    pub kind: String,
    pub major: String,
    pub platform: String,
    pub sha256: String,
    pub entry: String,
    pub installed_at: u64,
}

/// Install `rt` unless it is there, and return its executable and folder.
pub async fn ensure(ctx: &BuildCtx<'_>, rt: &PlannedRuntime) -> AppResult<(RuntimeExe, PathBuf)> {
    let offer = rt.offer.as_ref().ok_or_else(|| {
        AppError::invalid(format!(
            "No {} {} runtime is published for this computer.",
            rt.kind, rt.major
        ))
    })?;
    let entry = rel_path(&offer.entry).ok_or_else(|| {
        AppError::invalid(format!(
            "The runtime's entry {} is not a relative path.",
            offer.entry
        ))
    })?;

    let dir = ctx
        .data_root
        .join("runtimes")
        .join(&rt.kind)
        .join(&rt.major)
        .join(&offer.platform)
        .join(short(&offer.artifact.sha256));
    let found = |dir: &Path| -> Option<(RuntimeExe, PathBuf)> {
        let marker: RuntimeMarker =
            serde_json::from_slice(&std::fs::read(dir.join(MARKER)).ok()?).ok()?;
        let exe = dir.join("root").join(&entry);
        (marker.sha256 == offer.artifact.sha256 && exe.is_file()).then(|| {
            (
                RuntimeExe {
                    exe,
                    java: rt.kind == "java",
                },
                dir.join("root"),
            )
        })
    };
    if let Some(done) = found(&dir) {
        return Ok(done);
    }

    let partial = dir.with_file_name(format!("{}.partial", short(&offer.artifact.sha256)));
    if partial.exists() {
        std::fs::remove_dir_all(&partial)?;
    }
    std::fs::create_dir_all(&partial)?;
    let downloads = ctx.downloads_dir();
    std::fs::create_dir_all(&downloads)?;

    let result = install(ctx, rt, &partial, &downloads).await;
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(e);
    }

    let exe = partial.join("root").join(&entry);
    if !exe.is_file() {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(AppError::invalid(format!(
            "The {} {} runtime has no {}.",
            rt.kind, rt.major, offer.entry
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
    }
    let marker = RuntimeMarker {
        kind: rt.kind.clone(),
        major: rt.major.clone(),
        platform: offer.platform.clone(),
        sha256: offer.artifact.sha256.clone(),
        entry: offer.entry.clone(),
        installed_at: now_secs(),
    };
    let text = serde_json::to_vec_pretty(&marker)
        .map_err(|e| AppError::internal(format!("Could not write the runtime marker: {e}")))?;
    std::fs::write(partial.join(MARKER), text)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::rename(&partial, &dir)?;

    crate::audit!(
        ctx.audit,
        Info,
        Install,
        "runtime.installed",
        format!("Installed {} {} ({})", rt.kind, rt.major, offer.platform)
    );

    found(&dir).ok_or_else(|| AppError::internal("The runtime vanished after it was installed."))
}

async fn install(
    ctx: &BuildCtx<'_>,
    rt: &PlannedRuntime,
    partial: &Path,
    downloads: &Path,
) -> AppResult<()> {
    let offer = rt.offer.as_ref().expect("checked by ensure");
    let file = offer
        .artifact
        .url
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("runtime.tar.gz");
    let archive = format!("runtime-{}-{file}", short(&offer.artifact.sha256));

    let steps = parse_install_steps(&json!([
        { "action": "download", "artifact": "runtime", "to": { "root": "downloads", "path": archive } },
        {
            "action": "extract",
            "from": { "root": "downloads", "path": archive },
            "to": { "root": "layer", "path": "root" },
            "stripSingleRoot": offer.strip_single_root
        }
    ]))?;

    let mut hosts: Vec<String> = Vec::new();
    for url in std::iter::once(&offer.artifact.url).chain(&offer.artifact.mirrors) {
        if let Some(host) = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
        {
            hosts.push(host);
        }
    }
    let title = format!("{} {}", rt.kind, rt.major);
    let manifest = install_manifest(&format!("runtime/{}", rt.kind), &title, &hosts);
    let jail = Jail::build(&manifest, &install_roots(partial, downloads, None))?;
    let exec = Executor {
        manifest: &manifest,
        jail: &jail,
        http: ctx.http,
        audit: ctx.audit,
        downloads: ctx.downloads,
    };
    let env = InstallEnv {
        when: WhenContext::default(),
        artifacts: HashMap::from([("runtime".to_string(), offer.artifact.pin())]),
        ..Default::default()
    };

    let report = exec.run_install(&steps, &env).await;
    if report.run.ok {
        return Ok(());
    }
    Err(AppError::invalid(format!(
        "Installing {title} failed: {}",
        report.run.error.unwrap_or_else(|| "unknown error".into())
    )))
}

/// `bin/java` as a relative path, or `None` for an absolute one or one with
/// `..`.
fn rel_path(s: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = s.split('/').collect();
    if s.is_empty()
        || parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains('\\'))
    {
        return None;
    }
    Some(parts.iter().collect())
}
