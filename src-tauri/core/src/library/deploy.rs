//! Staging a mod into a sandbox, and putting that sandbox in front of the game.
//!
//! The join between four things that are deliberately separate everywhere else:
//! a **subscription** (the account wants this item), a **sandbox** (this profile
//! wants it at this priority), an **install rule** (where this game's mods go)
//! and the **deployment engine** (how files reach the game folder).
//!
//! TWO STEPS, NOT ONE
//! ------------------
//! ```text
//!   stage   — run the game's install rule with the sandbox's own staging
//!             folder standing in for the game directory
//!   deploy  — mirror every staged file into the real game folder, by
//!             whichever mechanism the sandbox is set to
//! ```
//!
//! Splitting them is what makes the whole feature work, and it is worth being
//! explicit about why:
//!
//!   * **Every install rule written before sandboxes existed still works.** A
//!     rule that copies to `mods/{fileName}` writes to
//!     `<staging>/<sandbox>/<mod>/mods/foo.jar`, and deployment puts
//!     `mods/foo.jar` in the game folder. The rule never learns which strategy
//!     is in use, and it should not — "where does a Minecraft mod go" and "how
//!     do files reach the game folder" have different right answers.
//!   * **Switching sandboxes is a link operation, not a download.** Staging
//!     survives an undeploy, so the mods for the profile you just left are
//!     still on disk when you come back.
//!   * **Nothing partially-downloaded is ever in the game folder.** A failed
//!     stage leaves a mess inside the app's own staging directory, which
//!     nothing else reads.
//!
//! The cost is disk: a direct-strategy sandbox holds each file twice, once in
//! staging and once in the game. That is the price of being able to undeploy
//! exactly, and it is why the linking strategies are the default.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::audit;
use crate::deploy::{self, DeployReport, DeployRequest, LedgerEntry, PurgeReport, Strategy};
use crate::error::{AppError, AppResult};
use crate::launch::{LaunchContext, LaunchOptions, LaunchPlan, VfsHandoff};
use crate::logging::Audit;
use crate::plugins::apps::{AppPluginKind, AppPlugins};
use crate::plugins::steps::{Executor, RunContext};
use crate::plugins::{jail_for, jail_for_scoped, JailRoots};
use crate::settings::AppSettings;

use super::db::{LibraryDb, LibraryEntry};
use super::sandbox::{Sandbox, SandboxMod};

/// Everything a sandbox operation needs that is not the sandbox.
pub struct SandboxCtx<'a> {
    pub plugins: &'a AppPlugins,
    pub roots: &'a JailRoots,
    pub settings: &'a AppSettings,
    pub http: &'a reqwest::Client,
    pub audit: &'a Audit,
    /// Platform directories for `execSearch` (`<programFiles>` and friends).
    pub exec_roots: &'a crate::detect::DetectRoots,
    /// The download queue, so staging a mod is visible and pausable.
    pub downloads: Option<&'a crate::download::DownloadManager>,
    /// Where every sandbox's staging folders live: `<staging>/<id>/<mod key>`.
    pub staging_root: &'a Path,
    /// Where imported mods live: `<local>/<local mod id>`.
    ///
    /// A DEVICE-wide directory, not a per-sandbox one, and that asymmetry is
    /// the point. A staged subscription is this sandbox's copy at this
    /// sandbox's pinned release; an imported mod is one set of files the user
    /// put on the machine once, and three sandboxes using it should be three
    /// references rather than three copies. Deployment only ever READS a mod's
    /// root, which is what makes sharing one safe — the same property that lets
    /// two sandboxes share a staging folder.
    pub local_root: &'a Path,
    /// Where displaced game files go: `<backups>/<id>/<relative path>`.
    pub backup_root: &'a Path,
    /// The app's data folder: install layers (`layers/`), runtimes
    /// (`runtimes/`), new isolated instances (`instances/`) and the patched
    /// overlay of a virtual deploy (`sandboxes/<id>/patched/`).
    pub data_root: &'a Path,
    /// Where a failed installer's log is copied.
    pub log_dir: &'a Path,
    /// Where install plans come from: the plan the catalog served (fresh or
    /// cached, `crate::catalog::plan_source`), or `None` for the compiled-in
    /// fallback.
    pub plan_source: Option<&'a crate::layers::plan::PlanSource>,
}

/// What staging one mod did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageOutcome {
    pub mod_key: String,
    pub ok: bool,
    pub rule: Option<String>,
    /// Files written into the staging folder.
    pub files: usize,
    pub error: Option<String>,
}

impl StageOutcome {
    fn failed(mod_key: &str, error: impl Into<String>) -> Self {
        Self {
            mod_key: mod_key.to_string(),
            ok: false,
            rule: None,
            files: 0,
            error: Some(error.into()),
        }
    }
}

/// Does this sandbox's game declare `isolatedInstance` in its `sandbox.json`?
///
/// Such a game's sandboxes are instances: each one owns a folder and none may
/// ever fall back to the app-wide game folder (the user's real install).
pub fn is_isolated(plugins: &AppPlugins, sandbox: &Sandbox) -> bool {
    sandbox
        .app_slug
        .as_deref()
        .and_then(|slug| plugins.sandbox_spec(slug))
        .is_some_and(|spec| spec.deploy.isolated_instance)
}

/// Make a fresh, empty instance folder: `<root>/<app slug>/<sandbox name>[-n]`.
///
/// The name is reduced to `[A-Za-z0-9 ._-]` first, so a sandbox called
/// `..\..\Windows` cannot walk out of `root`. An existing folder is never
/// reused (it may hold another sandbox's saves): a counter is appended.
pub fn create_instance_dir(root: &Path, slug: &str, name: &str) -> AppResult<PathBuf> {
    fn clean(raw: &str) -> String {
        let kept: String = raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-') {
                    c
                } else {
                    '-'
                }
            })
            .collect();

        let kept = kept.trim_matches(|c: char| c == '.' || c == ' ' || c == '-');

        if kept.is_empty() {
            "instance".to_owned()
        } else {
            kept.chars().take(64).collect()
        }
    }

    let parent = root.join(clean(slug));
    let base = clean(name);

    std::fs::create_dir_all(&parent).map_err(|e| {
        AppError::invalid(format!(
            "Could not create the instances folder {}: {e}",
            parent.display()
        ))
    })?;

    for n in 0..1000 {
        let leaf = if n == 0 {
            base.clone()
        } else {
            format!("{base}-{}", n + 1)
        };

        let dir = parent.join(leaf);

        // `create_dir` (not `_all`) fails when the folder exists, which is the
        // atomic "never reuse one" check.
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(AppError::invalid(format!(
                    "Could not create the instance folder {}: {e}",
                    dir.display()
                )))
            }
        }
    }

    Err(AppError::invalid(
        "Too many instance folders with that name. Rename the sandbox.",
    ))
}

/// Where a sandbox deploys to on this machine.
///
/// Its own folder when it has one — which is how CurseForge-style isolated
/// instances are expressed — and otherwise the app's configured directory for
/// the game, which is the "main install" every user starts with.
///
/// `isolated` is the game's `isolatedInstance` flag. Such a game NEVER falls
/// back to the app-wide folder: that is the user's real install (for Minecraft,
/// their `.minecraft`), and a sandbox is supposed to be separate from it.
///
/// A missing directory is an ERROR rather than something to create: creating it
/// means a typo in the settings produces an empty folder that the deploy then
/// "succeeds" into, leaving the user with a game that has no mods and an app
/// that says it worked.
pub fn target_dir(sandbox: &Sandbox, settings: &AppSettings, isolated: bool) -> AppResult<PathBuf> {
    let raw = if isolated {
        sandbox.game_dir.clone().ok_or_else(|| {
            AppError::invalid(
                "This sandbox has no instance folder of its own. This game's sandboxes are \
                 isolated instances and never use the game's main folder: choose an empty \
                 folder for it with Change.",
            )
        })?
    } else {
        sandbox
            .game_dir
            .clone()
            .or_else(|| settings.game_dirs.get(&sandbox.app_id.to_string()).cloned())
            .ok_or_else(|| {
                AppError::invalid(
                    "This sandbox has no game folder yet. Set one on the game's card in Library, \
                     or give the sandbox its own folder.",
                )
            })?
    };

    let path = PathBuf::from(raw);

    if !path.is_dir() {
        return Err(AppError::invalid(format!(
            "The folder this sandbox deploys into does not exist: {}",
            path.display()
        )));
    }

    Ok(path)
}

/// Which rule stages this item for this sandbox.
fn rule_kind(kind: &str) -> Option<AppPluginKind> {
    match kind {
        "mod" => Some(AppPluginKind::ManageMod),
        "asset" => Some(AppPluginKind::ManageAsset),
        "collection" => Some(AppPluginKind::ManageCollection),
        _ => None,
    }
}

/// Download and unpack one item into the sandbox's staging folder for it.
///
/// The item's own subscription row supplies the release; the sandbox supplies
/// the loader and game version a rule may match on. Nothing is written into the
/// game folder — that is [`deploy_sandbox`]'s job, and keeping the two apart is
/// what lets a failed download leave the game untouched.
pub async fn stage_mod(
    db: &LibraryDb,
    sandbox: &Sandbox,
    member: &SandboxMod,
    entry: &LibraryEntry,
    ctx: &SandboxCtx<'_>,
) -> StageOutcome {
    if !entry.installable {
        return StageOutcome::failed(
            &member.mod_key,
            "This item can no longer be installed by the app.",
        );
    }

    if entry.file_url.is_none() {
        return StageOutcome::failed(&member.mod_key, "This item has no release file yet.");
    }

    let Some(slug) = sandbox.app_slug.as_deref().or(entry.app_slug.as_deref()) else {
        return StageOutcome::failed(&member.mod_key, "This game has no plugin folder.");
    };

    let Some(kind) = rule_kind(&entry.kind) else {
        return StageOutcome::failed(&member.mod_key, "That is not something a sandbox holds.");
    };

    let file_name = super::install::safe_file_name(entry);

    let Some(rule) = ctx
        .plugins
        .choose(slug, kind, &file_name, sandbox.loader.as_deref())
    else {
        return StageOutcome::failed(
            &member.mod_key,
            "No install rule for this game — the app cannot place its files.",
        );
    };

    let Some(manage) = &rule.manage else {
        return StageOutcome::failed(&member.mod_key, "That rule declares no install steps.");
    };

    // The staging folder is this run's `gameDir`. Cleared first, so a re-stage
    // at a different version does not leave the old jar beside the new one —
    // the classic `cool-1.3.jar` + `cool-1.4.jar` crash.
    let stage = deploy::stage_dir(ctx.staging_root, sandbox.id, &member.mod_key);

    let _ = std::fs::remove_dir_all(&stage);

    if let Err(e) = std::fs::create_dir_all(&stage) {
        return StageOutcome::failed(
            &member.mod_key,
            format!("Could not prepare the staging folder: {e}"),
        );
    }

    let mut settings = ctx.settings.clone();

    settings.game_dirs.insert(
        entry.app_id.unwrap_or(sandbox.app_id).to_string(),
        stage.display().to_string(),
    );

    let manifest = rule.as_manifest();

    /*
     * `pluginData` is scoped per sandbox and per mod. Two sandboxes staging
     * the same mod run identical steps with an identical `{fileName}`, so a
     * shared scratch directory means the second run's download lands on the
     * first's — and a sandbox pinned to an older release quietly gets the newer
     * file.
     */
    let scope = format!("sb{}-{}", sandbox.id, member.mod_key);

    let jail = match jail_for_scoped(
        &manifest,
        ctx.roots,
        &settings,
        entry.app_id.or(Some(sandbox.app_id)),
        Some(&scope),
    ) {
        Ok(jail) => jail,
        Err(e) => {
            let _ = db.sandbox_mark_stage_failed(sandbox.id, &member.mod_key, &e.to_string());

            return StageOutcome::failed(&member.mod_key, e.to_string());
        }
    };

    let executor = Executor {
        manifest: &manifest,
        jail: &jail,
        http: ctx.http,
        audit: ctx.audit,
        downloads: ctx.downloads,
    };

    audit!(
        ctx.audit,
        Info,
        Install,
        "sandbox.stage.start",
        format!("{} → {} ({})", entry.name, sandbox.name, rule.source),
        plugin = rule.source
    );

    let run_ctx = stage_context(entry, sandbox, &file_name);

    let report = executor.run(&manage.install, &run_ctx).await;

    if report.ok {
        let _ = db.sandbox_mark_staged(
            sandbox.id,
            &member.mod_key,
            entry.latest_release_id,
            entry.latest_version.as_deref(),
        );

        audit!(
            ctx.audit,
            Info,
            Install,
            "sandbox.stage.ok",
            format!("{} → {}", entry.name, sandbox.name),
            plugin = rule.source
        );
    } else {
        let message = report
            .error
            .clone()
            .unwrap_or_else(|| "Staging did not finish.".into());

        let _ = db.sandbox_mark_stage_failed(sandbox.id, &member.mod_key, &message);

        audit!(
            ctx.audit,
            Error,
            Install,
            "sandbox.stage.fail",
            format!("{}: {message}", entry.name),
            plugin = rule.source
        );
    }

    StageOutcome {
        mod_key: member.mod_key.clone(),
        ok: report.ok,
        rule: Some(rule.source.clone()),
        files: report.applied.len(),
        error: report.error.clone(),
    }
}

/// The placeholder table a staging run interpolates.
fn stage_context(entry: &LibraryEntry, sandbox: &Sandbox, file_name: &str) -> RunContext {
    let mut map = std::collections::HashMap::new();

    map.insert("id".into(), entry.id.clone());
    map.insert("kind".into(), entry.kind.clone());
    map.insert("itemId".into(), entry.item_id.to_string());
    map.insert("itemName".into(), entry.name.clone());
    map.insert("webUrl".into(), entry.web_url.clone());
    map.insert("fileName".into(), file_name.to_string());

    if let Some(url) = &entry.file_url {
        map.insert("fileUrl".into(), url.clone());
    }
    if let Some(version) = &entry.latest_version {
        map.insert("version".into(), version.clone());
    }
    if let Some(release) = entry.latest_release_id {
        map.insert("releaseId".into(), release.to_string());
    }
    if let Some(slug) = &entry.app_slug {
        map.insert("appSlug".into(), slug.clone());
    }

    // The sandbox's own facts, under the names the pre-sandbox rules already
    // use — a rule written for `{installName}` keeps working.
    map.insert("installName".into(), sandbox.name.clone());
    map.insert("sandboxName".into(), sandbox.name.clone());
    map.insert("environment".into(), sandbox.environment.as_str().into());

    if let Some(version) = &sandbox.game_version {
        map.insert("gameVersion".into(), version.clone());
    }
    if let Some(loader) = &sandbox.loader {
        map.insert("loader".into(), loader.clone());
    }

    RunContext(map)
}

/// Delete one mod's staging folder.
pub fn unstage_mod(db: &LibraryDb, sandbox_id: i64, mod_key: &str, ctx: &SandboxCtx<'_>) {
    /*
     * An imported mod's files are NOT this sandbox's to delete. They live in
     * the device's local store and are very likely in another sandbox as well,
     * so removing one from a sandbox removes the row and leaves the files —
     * which is also what makes putting it back a click rather than a re-import.
     * Deleting an import for good is `local_delete`, which is a separate
     * decision and says so.
     */
    if crate::local::LocalMod::id_from_key(mod_key).is_none() {
        let stage = deploy::stage_dir(ctx.staging_root, sandbox_id, mod_key);

        let _ = std::fs::remove_dir_all(stage);
    }

    let _ = db.sandbox_clear_staged(sandbox_id, mod_key);
}

/// The mods a deploy will consider, in priority order.
///
/// Only the ones that are enabled AND staged. A mod that is in the list but has
/// never downloaded contributes nothing and must not be treated as a conflict
/// or as an empty mod — it is simply not ready yet, which the UI shows on the
/// row itself.
pub fn deployable(sandbox: &Sandbox, ctx: &SandboxCtx<'_>) -> Vec<deploy::DeployMod> {
    sandbox
        .enabled_mods()
        .filter(|m| m.staged_at.is_some())
        .map(|m| deploy::DeployMod {
            key: m.mod_key.clone(),
            name: m.name.clone(),
            root: mod_root(sandbox.id, &m.mod_key, ctx),
            priority: m.priority,
        })
        .collect()
}

/// The folder holding one member's files, laid out as they belong under the
/// game directory.
///
/// The one place the two sources of a mod meet, and deliberately the ONLY one:
/// everything downstream — the merge tree, the conflict report, the ledger,
/// the purge's "is this still ours" check — takes a root and does not ask where
/// it came from. An imported mod therefore conflicts with a subscribed one in
/// the same report, in the same load order, with no second code path to keep
/// in step.
fn mod_root(sandbox_id: i64, mod_key: &str, ctx: &SandboxCtx<'_>) -> PathBuf {
    match crate::local::LocalMod::id_from_key(mod_key) {
        Some(id) => crate::local::store::local_root(ctx.local_root, id),
        None => deploy::stage_dir(ctx.staging_root, sandbox_id, mod_key),
    }
}

// ------------------------------------------------------------ Install plan

/// The side a sandbox installs for: a server sandbox gets the server layer,
/// everything else the client one.
fn side_of(sandbox: &Sandbox) -> &'static str {
    match sandbox.environment {
        super::sandbox::Environment::Server => "server",
        _ => "client",
    }
}

/// The question this sandbox asks of a plan source, or `None` when it has no
/// game or game version yet. The shell asks the catalog (A2) for a served
/// plan with it before deploying.
///
/// `target` is the folder it deploys into, for `{targetDir}`; a sandbox with
/// no folder yet plans with an empty one, since no step of a layer may write
/// outside its layer.
pub fn plan_request(
    sandbox: &Sandbox,
    target: Option<&Path>,
) -> Option<crate::layers::plan::PlanRequest> {
    let (Some(slug), Some(game_version)) = (&sandbox.app_slug, &sandbox.game_version) else {
        return None;
    };
    Some(crate::layers::plan::PlanRequest {
        app_slug: slug.clone(),
        game_version: game_version.clone(),
        loader: sandbox.loader.clone(),
        loader_version: sandbox.loader_version.clone(),
        side: side_of(sandbox).to_string(),
        platform: crate::layers::plan::current_platform(),
        sandbox_id: sandbox.id.to_string(),
        sandbox_name: sandbox.name.clone(),
        target_dir: target.map(|t| t.display().to_string()).unwrap_or_default(),
    })
}

/// The install plan of this sandbox, or `None` when its game, version or
/// loader has no managed install (the deploy is then mods only, as before).
pub fn install_plan(
    sandbox: &Sandbox,
    ctx: &SandboxCtx<'_>,
    target: Option<&Path>,
) -> AppResult<Option<crate::layers::plan::InstallPlan>> {
    let Some(req) = plan_request(sandbox, target) else {
        return Ok(None);
    };
    let source = ctx
        .plan_source
        .unwrap_or_else(|| crate::layers::plan::PlanSource::compiled());
    crate::layers::plan::resolve(source, &req)
}

/// Give an isolated sandbox with no folder its own new instance folder under
/// `<data>/instances`, and return the sandbox as it now is.
fn ensure_instance_dir(
    db: &LibraryDb,
    sandbox: &Sandbox,
    ctx: &SandboxCtx<'_>,
) -> AppResult<Option<Sandbox>> {
    if sandbox.game_dir.is_some() || !is_isolated(ctx.plugins, sandbox) {
        return Ok(None);
    }

    let dir = create_instance_dir(
        &ctx.data_root.join("instances"),
        sandbox.app_slug.as_deref().unwrap_or("game"),
        &sandbox.name,
    )?;

    db.sandbox_patch(
        sandbox.id,
        &super::sandbox::SandboxPatch {
            game_dir: Some(Some(dir.display().to_string())),
            ..Default::default()
        },
    )?;

    audit!(
        ctx.audit,
        Info,
        Install,
        "sandbox.instance",
        format!("{} got its own folder {}", sandbox.name, dir.display())
    );

    db.sandbox_get(sandbox.id)
}

/// Build (or reuse) the plan's runtimes and layers, and return the layers as
/// pseudo-mods below every real mod: `i64::MIN + n`, so a mod always wins over
/// a layer and a later layer over an earlier one.
///
/// A dry run builds nothing: a layer that is already built is listed, one
/// that is not is a warning.
async fn layer_mods(
    plan: &crate::layers::plan::InstallPlan,
    ctx: &SandboxCtx<'_>,
    dry_run: bool,
    warnings: &mut Vec<String>,
) -> AppResult<Vec<deploy::DeployMod>> {
    use crate::layers::{build, checks::Limits, runtime};

    let bctx = build::BuildCtx {
        data_root: ctx.data_root,
        http: ctx.http,
        audit: ctx.audit,
        downloads: ctx.downloads,
        limits: Limits::from_settings(ctx.settings),
        log_dir: ctx.log_dir,
        // No proxy setting exists yet: an installer that needs one fails and
        // keeps its log.
        proxy: Vec::new(),
        on_output: None,
    };

    let mut runtimes = std::collections::HashMap::new();

    if !dry_run {
        for rt in plan.install_runtimes() {
            let (exe, root) = runtime::ensure(&bctx, rt).await?;
            runtimes.insert(rt.kind.clone(), (exe, root));
        }
    }

    let mut mods = Vec::new();

    for (n, layer) in plan.layers.iter().enumerate() {
        let root = if dry_run {
            let dir = bctx.layers_root(&plan.app_slug).join(&layer.layer_hash);

            if build::verify(&dir, layer).is_err() {
                warnings.push(format!("{} would be built first.", layer.title));
                continue;
            }

            dir.join("files")
        } else {
            build::ensure_layer(&bctx, plan, layer, &runtimes)
                .await?
                .files_dir
        };

        mods.push(deploy::DeployMod {
            key: format!("layer:{}", layer.manifest_key),
            name: layer.title.clone(),
            root,
            priority: i64::MIN + n as i64,
        });
    }

    Ok(mods)
}

/// The patched copies a virtual deploy maps over the real files.
pub fn patch_overlay(data_root: &Path, sandbox_id: i64) -> PathBuf {
    data_root
        .join("sandboxes")
        .join(sandbox_id.to_string())
        .join("patched")
}

/// Put a sandbox in front of the game.
///
/// With an install plan, in this order: consent, runtimes, layers (built
/// outside the game folder, so a failed build changes nothing there), the
/// merge with the layers as lowest-priority mods, then the JSON patches.
pub async fn deploy_sandbox(
    db: &LibraryDb,
    sandbox: &Sandbox,
    ctx: &SandboxCtx<'_>,
    dry_run: bool,
) -> AppResult<DeployReport> {
    let fresh = if dry_run {
        None
    } else {
        ensure_instance_dir(db, sandbox, ctx)?
    };
    let sandbox = fresh.as_ref().unwrap_or(sandbox);

    let target = target_dir(sandbox, ctx.settings, is_isolated(ctx.plugins, sandbox))?;

    /*
     * The game's own rules, when it has a `sandbox.json`. A game declaring
     * kernel anti-cheat, or simply not supporting a mechanism, is refused HERE
     * rather than after the deploy has half-linked a folder that EAC is about
     * to look at.
     */
    if let Some(slug) = &sandbox.app_slug {
        if let Some(spec) = ctx.plugins.sandbox_spec(slug) {
            if !spec.deploy.allows(sandbox.strategy.as_str()) {
                return Err(AppError::invalid(format!(
                    "{} does not support {} deployment. Supported here: {}.",
                    sandbox.app_name.as_deref().unwrap_or("This game"),
                    sandbox.strategy.as_str(),
                    if spec.deploy.supported_strategies.is_empty() {
                        "any".to_string()
                    } else {
                        spec.deploy.supported_strategies.join(", ")
                    }
                )));
            }

            // Injecting a DLL into a game is exactly what kernel anti-cheat
            // exists to stop, so this strategy is refused outright for such a
            // game, never merely warned about.
            if spec.deploy.kernel_anti_cheat() && sandbox.strategy == Strategy::Usvfs {
                return Err(AppError::invalid(format!(
                    "{} uses kernel-level anti-cheat, which treats the virtual filesystem's \
                     injected DLL as an attack. Use Direct.",
                    sandbox.app_name.as_deref().unwrap_or("This game")
                )));
            }
        }
    }

    // Resolve and consent first: a withdrawn build or an unanswered consent
    // stops the deploy before anything is downloaded.
    let plan = install_plan(sandbox, ctx, Some(&target))?;

    if let Some(plan) = &plan {
        crate::layers::consent::check(db, sandbox.id, plan)?;
    }

    let mut warnings = Vec::new();
    let mut mods = match &plan {
        Some(plan) => layer_mods(plan, ctx, dry_run, &mut warnings).await?,
        None => Vec::new(),
    };

    mods.extend(deployable(sandbox, ctx));

    let previous = db.sandbox_ledger(sandbox.id)?;
    let patches: &[crate::deploy::json_patch::JsonPatchSpec] =
        plan.as_ref().map_or(&[], |p| p.patches.as_slice());

    /*
     * The last deploy's patches are undone before this one merges, so the
     * patch step below always starts from the user's own file. A value the
     * user changed since is theirs and stays.
     */
    if !dry_run && previous.iter().any(LedgerEntry::is_json_patch) {
        let undone = deploy::json_patch::revert(&target, &previous);
        warnings.extend(undone.errors);
    }

    // A virtual deploy never edits the real file: it maps a patched copy over
    // it, above every mod.
    let virtual_patches = sandbox.strategy == Strategy::Usvfs && !patches.is_empty();

    if virtual_patches && !dry_run {
        let overlay = patch_overlay(ctx.data_root, sandbox.id);

        warnings.extend(deploy::json_patch::write_overlay(
            &target, &overlay, patches,
        ));

        mods.push(deploy::DeployMod {
            key: "patch:overlay".to_string(),
            name: "Install plan patches".to_string(),
            root: overlay,
            priority: i64::MAX,
        });
    }

    let staging = deploy::stage_root(ctx.staging_root, sandbox.id);
    let backups = deploy::backup_root(ctx.backup_root, sandbox.id);

    std::fs::create_dir_all(&staging)?;

    let blob = deploy::vfs_blob(ctx.staging_root, sandbox.id);

    let mut report = deploy::deploy(&DeployRequest {
        strategy: sandbox.strategy,
        target: &target,
        staging: &staging,
        backup_root: &backups,
        vfs_blob: Some(&blob),
        mods: &mods,
        previous: &previous,
        dry_run,
    })?;

    report.warnings.extend(warnings);

    if dry_run {
        return Ok(report);
    }

    if !virtual_patches && !patches.is_empty() {
        let (rows, errors) = deploy::json_patch::apply(&target, patches);

        report.ledger.extend(rows);
        report.errors.extend(errors);
    }

    /*
     * The ledger is written even when the deploy reported errors. It describes
     * what is ON DISK, and a partial deploy put files there — losing the record
     * of them would leave exactly the orphans the ledger exists to prevent.
     */
    db.sandbox_set_ledger(sandbox.id, &report.ledger)?;
    db.sandbox_mark_deployed(sandbox.id, &serde_json::to_value(&report)?)?;

    let state = plan.as_ref().map(|plan| crate::layers::InstallState {
        plan_hash: plan.plan_hash.clone(),
        layers: plan
            .layers
            .iter()
            .map(|l| crate::layers::InstallStateLayer {
                manifest_key: l.manifest_key.clone(),
                revision: l.revision,
                layer_hash: l.layer_hash.clone(),
            })
            .collect(),
        launch_vars: plan.launch_vars.clone(),
        deployed_at: crate::logging::now_rfc3339(),
    });

    db.sandbox_set_install_state(sandbox.id, state.as_ref())?;

    audit!(
        ctx.audit,
        Security,
        Install,
        "sandbox.deploy",
        format!(
            "{} → {} ({}; {} placed, {} reused, {} removed, {} backed up)",
            sandbox.name,
            target.display(),
            report.used,
            report.placed,
            report.reused,
            report.removed,
            report.backed_up
        )
    );

    Ok(report)
}

// ------------------------------------------------------------------- Launch

/// What launching a sandbox would run.
///
/// The sandbox's own game folder becomes the jail's `gameDir`, exactly as an
/// install's directory does — the same mechanism rather than a second one, so a
/// profile can never end up with laxer path checks than the main install.
///
/// The sandbox's `launchArgs` and `launchEnv` are appended AFTER the rule's,
/// for the reason a game's own argument parser gives: the last occurrence of a
/// repeated flag wins, so appending is what makes an override override.
pub fn launch_plan(
    db: &LibraryDb,
    sandbox: &Sandbox,
    ctx: &SandboxCtx<'_>,
) -> AppResult<LaunchPlan> {
    let target = target_dir(sandbox, ctx.settings, is_isolated(ctx.plugins, sandbox))?;

    let slug = sandbox.app_slug.as_deref().ok_or_else(|| {
        AppError::invalid("This sandbox's game has no slug, so no launch rule can be found for it.")
    })?;

    let rule = ctx
        .plugins
        .launch_for(slug, sandbox.loader.as_deref())
        .ok_or_else(|| {
            AppError::invalid(
                "This game has no launch rule, so the app cannot start it. Start it the way you \
                 normally would — the sandbox is deployed either way.",
            )
        })?;

    /*
     * The rule's jail is anchored at the SANDBOX's folder, which is why the
     * settings copy is patched rather than read: a sandbox with its own
     * directory is a different game folder from the app-wide one, and resolving
     * the executable against the wrong one would either fail or — worse —
     * launch the unmodded main install.
     */
    let mut settings = ctx.settings.clone();

    settings
        .game_dirs
        .insert(sandbox.app_id.to_string(), target.display().to_string());

    let manifest = rule.as_manifest();
    let jail = jail_for(&manifest, ctx.roots, &settings, Some(sandbox.app_id))?;

    let options: LaunchOptions = serde_json::from_value(serde_json::Value::Object(
        sandbox.options.clone().into_iter().collect(),
    ))
    .unwrap_or_default();

    /*
     * The virtual tree is attached only when the sandbox is BOTH set to that
     * strategy and has actually been deployed. A blob left over from a strategy
     * change would otherwise be carried into the game, mapping files from a
     * deploy that has since been undone.
     */
    let vfs = if sandbox.strategy == Strategy::Usvfs && sandbox.deployed_at.is_some() {
        let blob = deploy::vfs_blob(ctx.staging_root, sandbox.id);

        if !blob.is_file() {
            return Err(AppError::invalid(
                "This sandbox deploys virtually but has no published filesystem. Deploy it again \
                 before launching.",
            ));
        }

        Some(VfsHandoff {
            blob: blob.display().to_string(),
            root: target.display().to_string(),
        })
    } else {
        None
    };

    let launch_ctx = LaunchContext {
        install_name: Some(sandbox.name.clone()),
        game_version: sandbox.game_version.clone(),
        loader: sandbox.loader.clone(),
        install_dir: Some(target.clone()),
        game_dir: Some(target),
        vfs,
        exec_roots: ctx.exec_roots.clone(),
        path_dirs: crate::launch::system_path_dirs(),
        launch_vars: sandbox
            .install_state
            .as_ref()
            .map(|s| s.launch_vars.clone())
            .unwrap_or_default(),
    };

    let mut plan = crate::launch::plan(rule, &jail, &options, &launch_ctx)?;

    for arg in &sandbox.launch_args {
        // The same refusal the rule's own arguments get. A user-typed argument
        // is not more trusted than a plugin-supplied one.
        if arg.contains('\0') || arg.contains('\n') || arg.contains('\r') {
            return Err(AppError::jail(
                "A launch argument contains a control character.",
            ));
        }

        plan.args.push(arg.clone());
    }

    for (key, value) in &sandbox.launch_env {
        if key.contains('\0') || value.contains('\0') {
            return Err(AppError::jail(
                "A launch environment value contains a NUL byte.",
            ));
        }

        plan.env.insert(key.clone(), value.clone());
    }

    /*
     * Advisory, and deliberately not an error. A user who wants to start the
     * game before deploying is entitled to; what they are not entitled to is
     * doing it without being told, because the symptom — a game with none of
     * the sandbox's mods — looks exactly like the mods being broken.
     */
    if sandbox.needs_deploy(&db.sandbox_ledger(sandbox.id)?) {
        audit!(
            ctx.audit,
            Warn,
            Install,
            "sandbox.launch.stale",
            format!("{} has changes that are not deployed", sandbox.name)
        );
    }

    Ok(plan)
}

/// Take a sandbox back out of the game folder.
pub fn purge_sandbox(
    db: &LibraryDb,
    sandbox: &Sandbox,
    ctx: &SandboxCtx<'_>,
) -> AppResult<PurgeReport> {
    let target = target_dir(sandbox, ctx.settings, is_isolated(ctx.plugins, sandbox))?;
    let previous = db.sandbox_ledger(sandbox.id)?;

    let mut report = deploy::purge(&target, &previous);

    // The install plan's patches: undone where the value is still ours, left
    // where the user changed it since. Either way the row is done with.
    let undone = deploy::json_patch::revert(&target, &previous);
    report.errors.extend(undone.errors);

    let overlay = patch_overlay(ctx.data_root, sandbox.id);

    if overlay.exists() {
        if let Err(e) = std::fs::remove_dir_all(&overlay) {
            tracing::warn!("could not remove the patch overlay for {}: {e}", sandbox.id);
        }
    }

    /*
     * The published tree goes too, unconditionally rather than only for a
     * sandbox currently set to `usvfs`. A user who deployed virtually, switched
     * the strategy and then undeployed would otherwise leave a blob behind that
     * the next launch of that game would still be handed.
     */
    if let Err(e) = deploy::purge_vfs(&deploy::vfs_blob(ctx.staging_root, sandbox.id)) {
        tracing::warn!("could not remove the virtual tree for {}: {e}", sandbox.id);
    }

    /*
     * Only the rows that were actually dealt with are dropped. A file left in
     * place because the user edited it is still a file we put there, so its
     * ledger row survives — otherwise the next deploy would treat it as
     * somebody else's file and move it into the backup store.
     */
    let kept: std::collections::HashSet<&str> = report.kept.iter().map(String::as_str).collect();

    let remaining: Vec<LedgerEntry> = previous
        .into_iter()
        .filter(|e| !e.is_json_patch() && kept.contains(e.path.as_str()))
        .collect();

    db.sandbox_set_ledger(sandbox.id, &remaining)?;

    if remaining.is_empty() {
        db.sandbox_mark_purged(sandbox.id)?;
        db.sandbox_set_install_state(sandbox.id, None)?;
    }

    audit!(
        ctx.audit,
        Security,
        Install,
        "sandbox.purge",
        format!(
            "{} ← {} ({} removed, {} restored, {} left alone)",
            sandbox.name,
            target.display(),
            report.removed,
            report.restored,
            report.kept.len()
        )
    );

    Ok(report)
}

/// Check a sandbox's deployment against the disk.
pub fn verify_sandbox(
    db: &LibraryDb,
    sandbox: &Sandbox,
    ctx: &SandboxCtx<'_>,
) -> AppResult<deploy::VerifyReport> {
    let target = target_dir(sandbox, ctx.settings, is_isolated(ctx.plugins, sandbox))?;
    let ledger = db.sandbox_ledger(sandbox.id)?;

    Ok(deploy::verify(&target, &ledger))
}

/// Which deployment strategies this sandbox could actually use here.
///
/// Both halves have to agree: the machine has to be able to make the link (the
/// engine's probe) and the game has to permit the mechanism (its `sandbox.json`).
pub fn strategies_for(
    sandbox: &Sandbox,
    ctx: &SandboxCtx<'_>,
) -> AppResult<Vec<deploy::StrategyReport>> {
    let target = target_dir(sandbox, ctx.settings, is_isolated(ctx.plugins, sandbox))?;
    let staging = deploy::stage_root(ctx.staging_root, sandbox.id);

    std::fs::create_dir_all(&staging)?;

    let mut listed = deploy::available_strategies(&staging, &target);

    let spec = sandbox
        .app_slug
        .as_deref()
        .and_then(|slug| ctx.plugins.sandbox_spec(slug));

    if let Some(spec) = spec {
        for report in &mut listed {
            if !spec.deploy.allows(&report.strategy) {
                report.available = false;
                report.reason = Some(format!(
                    "{} does not support this method.",
                    sandbox.app_name.as_deref().unwrap_or("This game")
                ));
            } else if spec.deploy.kernel_anti_cheat()
                && Strategy::parse(&report.strategy) == Some(Strategy::Usvfs)
            {
                report.available = false;
                report.reason = Some(format!(
                    "{} uses kernel-level anti-cheat, which treats an injected DLL as an \
                     attack. Use Direct.",
                    sandbox.app_name.as_deref().unwrap_or("This game")
                ));
            } else if spec.deploy.kernel_anti_cheat()
                && Strategy::parse(&report.strategy).is_some_and(|s| !s.modifies_game_files())
            {
                /*
                 * Still offered, but never silently. Kernel anti-cheat watches
                 * for a game folder whose files are not where they should be,
                 * and the cost of being wrong is somebody's account rather than
                 * a failed install — so the warning rides on the option itself.
                 */
                report.reason = Some(format!(
                    "{} uses kernel-level anti-cheat. Linked files can be treated as \
                     tampering — Direct is the safe choice.",
                    sandbox.app_name.as_deref().unwrap_or("This game")
                ));
            }
        }
    }

    Ok(listed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::sandbox::{Environment, NewSandbox};
    use std::collections::BTreeMap;

    fn settings_with(app_id: i64, dir: &Path) -> AppSettings {
        let mut settings = AppSettings::default();

        settings
            .game_dirs
            .insert(app_id.to_string(), dir.display().to_string());

        settings
    }

    fn new_sandbox() -> NewSandbox {
        NewSandbox {
            app_id: 1,
            app_slug: Some("minecraft".into()),
            app_name: Some("Minecraft".into()),
            name: "Test".into(),
            description: None,
            environment: Environment::Client,
            strategy: Strategy::Direct,
            game_version: None,
            loader: None,
            loader_version: None,
            preset: None,
            game_dir: None,
            options: BTreeMap::new(),
            cloud_sync: false,
            auto_update: true,
        }
    }

    #[test]
    fn a_sandbox_without_a_game_folder_says_so_rather_than_guessing() {
        let db = LibraryDb::open_memory().expect("db");
        let id = db.sandbox_create(&new_sandbox()).expect("create");
        let sandbox = db.sandbox_get(id).expect("get").expect("present");

        let err = target_dir(&sandbox, &AppSettings::default(), false).expect_err("must refuse");

        assert!(err.to_string().contains("no game folder"));
    }

    #[test]
    fn an_isolated_game_never_falls_back_to_the_app_wide_folder() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let real = tmp.path().join(".minecraft");

        std::fs::create_dir_all(&real).expect("mkdir");

        let db = LibraryDb::open_memory().expect("db");
        let id = db.sandbox_create(&new_sandbox()).expect("create");
        let sandbox = db.sandbox_get(id).expect("get").expect("present");

        let err = target_dir(&sandbox, &settings_with(1, &real), true).expect_err("must refuse");

        assert!(err.to_string().contains("instance folder"), "{err}");
    }

    #[test]
    fn the_shipped_minecraft_sandboxes_are_isolated() {
        let plugins = AppPlugins::shipped();
        let db = LibraryDb::open_memory().expect("db");
        let id = db.sandbox_create(&new_sandbox()).expect("create");
        let mut sandbox = db.sandbox_get(id).expect("get").expect("present");

        sandbox.app_slug = Some("minecraft".into());
        assert!(is_isolated(&plugins, &sandbox));

        sandbox.app_slug = Some("gtav".into());
        assert!(!is_isolated(&plugins, &sandbox));
    }

    #[test]
    fn instance_folders_are_fresh_sanitised_and_inside_the_root() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let a = create_instance_dir(tmp.path(), "minecraft", "Test").expect("a");
        let b = create_instance_dir(tmp.path(), "minecraft", "Test").expect("b");
        let c = create_instance_dir(tmp.path(), "minecraft", "..\\..\\Windows/x").expect("c");

        assert_eq!(a, tmp.path().join("minecraft/Test"));
        assert_eq!(b, tmp.path().join("minecraft/Test-2"));
        assert!(a.is_dir() && b.is_dir());
        assert!(c.starts_with(tmp.path().join("minecraft")));
        assert_eq!(c.parent(), Some(tmp.path().join("minecraft").as_path()));
    }

    #[test]
    fn a_sandbox_folder_overrides_the_apps_configured_one() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let shared = tmp.path().join("shared");
        let own = tmp.path().join("own");

        std::fs::create_dir_all(&shared).expect("mkdir");
        std::fs::create_dir_all(&own).expect("mkdir");

        let db = LibraryDb::open_memory().expect("db");

        let mut new = new_sandbox();
        new.game_dir = Some(own.display().to_string());

        let id = db.sandbox_create(&new).expect("create");
        let sandbox = db.sandbox_get(id).expect("get").expect("present");

        let resolved = target_dir(&sandbox, &settings_with(1, &shared), false).expect("resolved");

        assert_eq!(resolved, own);
    }

    #[test]
    fn a_configured_folder_that_is_gone_is_an_error() {
        let db = LibraryDb::open_memory().expect("db");
        let id = db.sandbox_create(&new_sandbox()).expect("create");
        let sandbox = db.sandbox_get(id).expect("get").expect("present");

        let settings = settings_with(1, Path::new("/definitely/not/here"));

        assert!(target_dir(&sandbox, &settings, false).is_err());
    }

    /// A mod in the list but not yet downloaded is not deployable, and must not
    /// register as an empty mod or as a conflict.
    #[test]
    fn only_staged_and_enabled_mods_are_deployable() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = LibraryDb::open_memory().expect("db");

        let id = db.sandbox_create(&new_sandbox()).expect("create");

        let staged = db.sandbox_add_mod(id, "mod", 1, "Staged").expect("add");
        let unstaged = db.sandbox_add_mod(id, "mod", 2, "Not yet").expect("add");
        let disabled = db.sandbox_add_mod(id, "mod", 3, "Off").expect("add");

        db.sandbox_mark_staged(id, &staged, Some(1), None)
            .expect("staged");
        db.sandbox_mark_staged(id, &disabled, Some(1), None)
            .expect("staged");
        db.sandbox_set_mod_enabled(id, &disabled, false)
            .expect("disable");

        let sandbox = db.sandbox_get(id).expect("get").expect("present");

        let audit = Audit::new(tmp.path().join("audit.jsonl"));
        let roots = JailRoots {
            data: tmp.path().join("data"),
            cache: tmp.path().join("cache"),
        };
        let http = crate::http::client();
        let plugins = AppPlugins::default();
        let settings = AppSettings::default();

        let ctx = SandboxCtx {
            plugins: &plugins,
            roots: &roots,
            settings: &settings,
            http: &http,
            audit: &audit,
            exec_roots: &crate::detect::DetectRoots::default(),
            downloads: None,
            staging_root: &tmp.path().join("staging"),
            backup_root: &tmp.path().join("backups"),
            local_root: &tmp.path().join("local-mods"),
            data_root: tmp.path(),
            log_dir: tmp.path(),
            plan_source: None,
        };

        let mods = deployable(&sandbox, &ctx);

        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].key, staged);
        assert_ne!(mods[0].key, unstaged);
    }

    /// Injecting a DLL is what kernel anti-cheat exists to stop: the virtual
    /// filesystem is refused for such a game, and shown disabled in the picker.
    #[tokio::test]
    async fn kernel_anti_cheat_refuses_the_virtual_filesystem() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_root = tmp.path().join("plugins");
        let own = tmp.path().join("own");

        std::fs::create_dir_all(&own).expect("mkdir");
        std::fs::create_dir_all(plugin_root.join("app/minecraft")).expect("mkdir");
        std::fs::write(
            plugin_root.join("app/minecraft/sandbox.json"),
            r#"{
                "manifestVersion": 1,
                "sandbox": {
                    "deploy": {
                        "antiCheat": "kernel",
                        "defaultStrategy": "direct",
                        "supportedStrategies": ["direct", "usvfs"]
                    }
                }
            }"#,
        )
        .expect("write");

        let db = LibraryDb::open_memory().expect("db");

        let mut new = new_sandbox();
        new.strategy = Strategy::Usvfs;
        new.game_dir = Some(own.display().to_string());

        let id = db.sandbox_create(&new).expect("create");
        let sandbox = db.sandbox_get(id).expect("get").expect("present");

        let audit = Audit::new(tmp.path().join("audit.jsonl"));
        let roots = JailRoots {
            data: tmp.path().join("data"),
            cache: tmp.path().join("cache"),
        };
        let http = crate::http::client();
        let plugins = AppPlugins::load(&plugin_root);
        let settings = AppSettings::default();

        let ctx = SandboxCtx {
            plugins: &plugins,
            roots: &roots,
            settings: &settings,
            http: &http,
            audit: &audit,
            exec_roots: &crate::detect::DetectRoots::default(),
            downloads: None,
            staging_root: &tmp.path().join("staging"),
            backup_root: &tmp.path().join("backups"),
            local_root: &tmp.path().join("local-mods"),
            data_root: tmp.path(),
            log_dir: tmp.path(),
            plan_source: None,
        };

        let err = deploy_sandbox(&db, &sandbox, &ctx, true)
            .await
            .expect_err("must refuse");

        assert!(err.to_string().contains("anti-cheat"), "{err}");

        let listed = strategies_for(&sandbox, &ctx).expect("listed");

        let usvfs = listed
            .iter()
            .find(|r| Strategy::parse(&r.strategy) == Some(Strategy::Usvfs))
            .expect("usvfs is listed");

        assert!(!usvfs.available);
        assert!(usvfs.reason.as_deref().unwrap_or("").contains("anti-cheat"));
    }
}
