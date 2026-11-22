//! Executes an installer plugin's plan.
//!
//! Every step is audited before and after, every path goes through the
//! [`Jail`], and every network fetch is bounded and host-checked. There is
//! no step that runs a program, so the worst a hostile manifest can achieve is
//! writing junk into the directories the user explicitly granted — which is
//! bad, recoverable, and fully recorded.
//!
//! **Not transactional.** A plan that fails at step 7 leaves steps 1–6 applied.
//! A real rollback would mean shadowing the game directory, which is
//! prohibitively expensive for a multi-gigabyte install; instead every applied
//! step is journalled (see [`RunReport::applied`]) so the uninstall plan — and
//! the user reading the log — knows exactly what landed.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::audit;
use crate::error::{AppError, AppResult};
use crate::logging::Audit;
use crate::plugins::jail::Jail;
use crate::plugins::manifest::{Manifest, PathRef, Step};
use crate::plugins::steps_install::{self, InstallRun};

/// Hard cap on a single download, when the manifest does not set a smaller one.
const DEFAULT_MAX_DOWNLOAD: u64 = 2 * 1024 * 1024 * 1024;

/// Cap on total extracted bytes, per extract step. A zip bomb is a 40 KB file
/// that becomes 5 GB; without this the jail's path checks are irrelevant
/// because the disk fills before anything escapes.
const MAX_EXTRACT_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Cap on entry count, for the same reason at the other extreme: millions of
/// empty files.
const MAX_EXTRACT_ENTRIES: usize = 50_000;

/// Cap on a JSON config the plugin patches.
const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub ok: bool,
    pub steps_total: usize,
    pub steps_run: usize,
    /// Paths this run created or modified, in order. The uninstall side and the
    /// activity log both read this.
    pub applied: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Values a plan may interpolate with `{name}`.
///
/// Substitution is a plain key lookup, never an expression: a template language
/// here would be a scripting language with extra steps.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RunContext(pub HashMap<String, String>);

impl RunContext {
    /// One value, when the context has it.
    fn get(&self, key: &str) -> Option<&String> {
        self.0.get(key)
    }

    /// The checksum to enforce: the step's own, else the server's for the release file
    /// when the step downloads exactly that file.
    pub(crate) fn expected_sha(&self, url: &str, step_sha: Option<&str>) -> Option<String> {
        if let Some(sha) = step_sha {
            return Some(sha.to_string());
        }

        let sha = self.get("fileSha256")?.trim();

        (!sha.is_empty() && self.get("fileUrl").is_some_and(|f| f == url)).then(|| sha.to_string())
    }

    /// Replace `{key}` occurrences. An unknown key is left as-is rather than
    /// erroring — the URL check afterwards is what decides whether the result
    /// is acceptable, and a literal `{foo}` is never a valid host.
    fn fill(&self, template: &str) -> String {
        let mut out = template.to_string();

        for (key, value) in &self.0 {
            out = out.replace(&format!("{{{key}}}"), value);
        }

        out
    }
}

/// The message of a digest mismatch from a direct download, kept as a constant
/// because mirror failover recognises it by value.
const CHECKSUM_MISMATCH: &str = "Downloaded file did not match the expected checksum.";

/// Whether another mirror may be tried after `e`: a network failure, or the
/// bytes arriving with the wrong digest. Everything else (a refused URL, a
/// cancel) is final.
fn failover_worthy(e: &AppError) -> bool {
    match e {
        AppError::Network(_) => true,
        AppError::Jail(m) => m == CHECKSUM_MISMATCH,
        _ => false,
    }
}

/// Exactly 64 hex digits.
pub(crate) fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub struct Executor<'a> {
    pub manifest: &'a Manifest,
    pub jail: &'a Jail,
    pub http: &'a reqwest::Client,
    pub audit: &'a Audit,

    /// The download queue, when there is one.
    ///
    /// With it, a `download` step becomes a row in the same queue as everything
    /// else: the same progress bar, the same bandwidth limit, the same pause
    /// button, and a resume that survives the app closing. A step that streams
    /// its own bytes is invisible to all four.
    ///
    /// `None` falls back to streaming inline, which is what the tests use and
    /// what a caller with no queue to hand gets. The host allow-list, the size
    /// cap and the checksum are enforced on both paths — the queue does not
    /// replace any check, it only moves where the bytes are read.
    pub downloads: Option<&'a crate::download::DownloadManager>,
}

impl Executor<'_> {
    pub async fn run(&self, steps: &[Step], ctx: &RunContext) -> RunReport {
        let mut applied: Vec<String> = Vec::new();

        for (index, step) in steps.iter().enumerate() {
            let label = step_label(step);

            audit!(
                self.audit,
                Info,
                Install,
                "plugin.step.start",
                format!("[{}/{}] {label}", index + 1, steps.len()),
                plugin = self.manifest.id
            );

            match self.run_step(step, ctx, &mut applied, None).await {
                Ok(()) => audit!(
                    self.audit,
                    Info,
                    Install,
                    "plugin.step.ok",
                    format!("[{}/{}] {label}", index + 1, steps.len()),
                    plugin = self.manifest.id
                ),
                Err(err) => {
                    audit!(
                        self.audit,
                        Error,
                        Install,
                        "plugin.step.fail",
                        format!("[{}/{}] {label}: {}", index + 1, steps.len(), err.detail()),
                        plugin = self.manifest.id
                    );

                    return RunReport {
                        ok: false,
                        steps_total: steps.len(),
                        steps_run: index,
                        applied,
                        failed_at: Some(index),
                        error: Some(err.to_string()),
                    };
                }
            }
        }

        RunReport {
            ok: true,
            steps_total: steps.len(),
            steps_run: steps.len(),
            applied,
            failed_at: None,
            error: None,
        }
    }

    /// Resolve a step's path, with its placeholders filled.
    ///
    /// **Filled BEFORE the jail resolves it, never after.** Every lexical
    /// rejection in `join_relative` — `../`, an absolute path, a drive prefix,
    /// a NUL — then applies to the SUBSTITUTED value, so an item whose name is
    /// `../../../etc` is refused by exactly the check that refuses a literal
    /// one. Filling afterwards would mean checking a path and then building a
    /// different one, which is the shape of every path-traversal bug there is.
    ///
    /// Paths need this as much as URLs do: a rule that writes
    /// `mods/{fileName}` is how nearly every install rule names its output, and
    /// without substitution it produces a file called `{fileName}` — one file
    /// that every mod for that game then overwrites.
    pub(crate) fn resolve(&self, path_ref: &PathRef, ctx: &RunContext, write: bool) -> AppResult<PathBuf> {
        self.jail.resolve(
            &PathRef {
                root: path_ref.root,
                path: ctx.fill(&path_ref.path),
            },
            write,
        )
    }

    /// One step. `install` is `Some` only for an install manifest's run (see
    /// [`Executor::run_install`]); the install-only steps refuse to run without
    /// it, so a plugin plan that slipped past the parser still cannot reach them.
    pub(crate) async fn run_step(
        &self,
        step: &Step,
        ctx: &RunContext,
        applied: &mut Vec<String>,
        install: Option<&InstallRun<'_>>,
    ) -> AppResult<()> {
        match step {
            Step::Download { artifact: Some(_), .. } => {
                // `run_install` replaces an artifact download with its binding
                // before it gets here; anything else is a plugin naming one.
                return Err(AppError::invalid("download.artifact is not allowed in plugins"));
            }

            Step::Download {
                url,
                to,
                sha256,
                max_bytes,
                mirrors,
                artifact: None,
            } => {
                let target = self.resolve(to, ctx, true)?;
                let resolved = ctx.fill(url);

                if mirrors.is_empty() {
                    self.download(
                        &resolved,
                        &target,
                        ctx.expected_sha(&resolved, sha256.as_deref()).as_deref(),
                        *max_bytes,
                        ctx,
                        0,
                    )
                    .await?;
                } else {
                    let all: Vec<String> = std::iter::once(resolved)
                        .chain(mirrors.iter().map(|m| ctx.fill(m)))
                        .collect();

                    self.download_mirrored(&all, &target, sha256.as_deref(), *max_bytes, ctx)
                        .await?;
                }

                applied.push(display(&target));
            }

            Step::Extract {
                from,
                to,
                strip,
                strip_single_root,
                include,
            } => {
                let source = self.resolve(from, ctx, false)?;
                let dest = self.resolve(to, ctx, true)?;

                let written = extract_archive(
                    &source,
                    &dest,
                    &ExtractOpts {
                        strip: *strip,
                        strip_single_root: *strip_single_root,
                        include,
                    },
                )?;

                audit!(
                    self.audit,
                    Info,
                    Install,
                    "plugin.extract",
                    format!("{} entries → {}", written.len(), display(&dest)),
                    plugin = self.manifest.id
                );

                /*
                 * Every extracted FILE, not the destination directory.
                 *
                 * The journal is what `library::install::run_uninstall` sweeps,
                 * and it sweeps by removing each recorded path — a directory
                 * with `remove_dir_all`. Recording `Data` for a Skyrim mod, or
                 * `scripts` for a GTA V one, therefore meant uninstalling one
                 * mod deleted the shared folder every other mod had also
                 * extracted into. The GTA V rule's own comment says it relies
                 * on the journal for exactly this, which is what makes the
                 * mismatch worth fixing here rather than in the rules: a rule
                 * cannot enumerate what an archive contains, and this can.
                 *
                 * It is capped by MAX_EXTRACT_ENTRIES, so the row is bounded.
                 * A large texture pack does produce a large list, and that is
                 * the correct size for a record of what it wrote.
                 */
                for path in &written {
                    applied.push(display(path));
                }
            }

            Step::Copy { from, to } => {
                let source = self.resolve(from, ctx, false)?;
                let dest = self.resolve(to, ctx, true)?;

                ensure_parent(&dest)?;
                std::fs::copy(&source, &dest)?;

                applied.push(display(&dest));
            }

            Step::Move { from, to } => {
                let source = self.resolve(from, ctx, true)?;
                let dest = self.resolve(to, ctx, true)?;

                ensure_parent(&dest)?;

                /*
                 * `rename` fails across filesystems, which is the normal case
                 * when the download cache and the game directory are on
                 * different volumes. Fall back to copy-then-delete rather than
                 * surfacing an EXDEV the user cannot act on.
                 */
                if std::fs::rename(&source, &dest).is_err() {
                    std::fs::copy(&source, &dest)?;
                    std::fs::remove_file(&source)?;
                }

                applied.push(display(&dest));
            }

            Step::Mkdir { path } => {
                let dir = self.resolve(path, ctx, true)?;
                std::fs::create_dir_all(&dir)?;

                applied.push(display(&dir));
            }

            Step::Remove { path } => {
                let target = self.resolve(path, ctx, true)?;

                if target.is_dir() {
                    std::fs::remove_dir_all(&target)?;
                } else if target.exists() {
                    std::fs::remove_file(&target)?;
                }

                applied.push(format!("removed {}", display(&target)));
            }

            Step::WriteText {
                path,
                content,
                if_missing,
            } => {
                let target = self.resolve(path, ctx, true)?;

                // A stub that must not clobber what is already there.
                if *if_missing && target.exists() {
                    return Ok(());
                }

                ensure_parent(&target)?;
                std::fs::write(&target, ctx.fill(content))?;

                applied.push(display(&target));
            }

            Step::PatchJson {
                path,
                pointer,
                value,
            } => {
                let target = self.resolve(path, ctx, true)?;

                self.patch_json(&target, pointer, value)?;

                applied.push(display(&target));
            }

            Step::RunInstaller { .. }
            | Step::DownloadList { .. }
            | Step::MergeJson { .. }
            | Step::RequireFile { .. } => match install {
                Some(run) => steps_install::run(self, step, run, applied).await?,
                None => {
                    return Err(AppError::invalid(format!(
                        "{} is not allowed in plugins",
                        step.action_name()
                    )))
                }
            },
        }

        Ok(())
    }

    // ------------------------------------------------------------- Download

    /// The https and allow-list gate every download URL passes, primary or
    /// mirror, after placeholder substitution. One function so a mirror can
    /// never be held to a looser standard than the first URL.
    pub(crate) fn check_url(&self, url: &str) -> AppResult<url::Url> {
        let parsed = url::Url::parse(url)
            .map_err(|_| AppError::jail(format!("'{url}' is not a valid URL.")))?;

        // Plaintext would let anyone on the path replace a mod jar with
        // anything they like, checksum or no checksum (they would rewrite that
        // too — it comes from the same manifest, over the same channel).
        if parsed.scheme() != "https" {
            return Err(AppError::jail("Downloads must use https."));
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| AppError::jail("Download URL has no host."))?;

        /*
         * Re-checked AFTER placeholder substitution, which is the whole reason
         * the check lives here rather than at manifest-validation time: a
         * template of `https://{mirror}/file.zip` is not checkable until
         * `{mirror}` is known.
         */
        if !self.manifest.allows_host(host) {
            return Err(AppError::jail(format!(
                "'{host}' is not in this plugin's allowed download hosts."
            )));
        }

        Ok(parsed)
    }

    /// A download step with mirrors: the first URL, then each mirror in order.
    ///
    /// Every URL is gated BEFORE any byte moves, so a bad mirror is a refusal
    /// of the whole step and not something discovered only once the primary
    /// happens to fail. The pinned digest is the step's own `sha256`, required
    /// and well-formed: a mirror is somebody else's server, and the digest is
    /// the only thing that makes its bytes the author's bytes.
    ///
    /// Failover happens on a network failure or a digest mismatch, and on
    /// nothing else. A jail refusal, a cancel or a pause ends the step, and no
    /// attempt ever runs with a check the first one did not.
    async fn download_mirrored(
        &self,
        urls: &[String],
        target: &Path,
        pinned: Option<&str>,
        max_bytes: Option<u64>,
        ctx: &RunContext,
    ) -> AppResult<()> {
        let pinned = pinned
            .map(str::trim)
            .filter(|s| is_sha256_hex(s))
            .ok_or_else(|| {
                AppError::jail("A download with mirrors needs a pinned 64-hex sha256.")
            })?;

        for url in urls {
            self.check_url(url)?;
        }

        let mut last = None;

        for (attempt, url) in urls.iter().enumerate() {
            match self
                .download(url, target, Some(pinned), max_bytes, ctx, attempt)
                .await
            {
                Ok(()) => return Ok(()),
                Err(e) if failover_worthy(&e) => {
                    audit!(
                        self.audit,
                        Warn,
                        Network,
                        "plugin.download.failover",
                        format!("{url}: {e}"),
                        plugin = self.manifest.id
                    );

                    // A half-written or mismatching file never stays for the
                    // next attempt to resume over.
                    let _ = std::fs::remove_file(target);
                    last = Some(e);
                }
                Err(e) => return Err(e),
            }
        }

        Err(last.unwrap_or_else(|| AppError::invalid("No download URL.")))
    }

    pub(crate) async fn download(
        &self,
        url: &str,
        target: &Path,
        expect_sha: Option<&str>,
        max_bytes: Option<u64>,
        ctx: &RunContext,
        attempt: usize,
    ) -> AppResult<()> {
        let parsed = self.check_url(url)?;

        let cap = max_bytes
            .unwrap_or(DEFAULT_MAX_DOWNLOAD)
            .min(DEFAULT_MAX_DOWNLOAD);

        audit!(
            self.audit,
            Security,
            Network,
            "plugin.download",
            format!("{url} → {}", display(target)),
            plugin = self.manifest.id
        );

        if let Some(queue) = self.downloads {
            ensure_parent(target)?;

            /*
             * A DETERMINISTIC id, from the plugin and the destination. Two runs
             * of the same step are then the same queue row rather than two
             * writers for one file — which is what re-running a failed install
             * does, and what two sandboxes staging the same mod would do if the
             * destination were shared.
             */
            let mut id = format!("plugin:{}:{}", self.manifest.id, display(target));

            // A mirror is a different transfer: sharing the failed one's id
            // would hand back its terminal state instead of trying again.
            if attempt > 0 {
                id.push_str(&format!("#mirror{attempt}"));
            }

            return queue
                .run_to_completion(crate::download::DownloadRequest {
                    id,
                    url: url.to_string(),
                    dest: target.to_path_buf(),
                    /*
                     * The ITEM's name, not the file's. A queue row reading
                     * `a1b2c3d4.zip` is one the user cannot connect to
                     * anything they asked for — release files are frequently
                     * named by hash or by build number, and the queue is the
                     * screen where somebody looks to find out what is taking
                     * so long.
                     */
                    label: ctx
                        .get("itemName")
                        .cloned()
                        .or_else(|| target.file_name().map(|n| n.to_string_lossy().into_owned()))
                        .unwrap_or_else(|| self.manifest.name.clone()),
                    sha256: expect_sha.map(str::to_string),
                    size_hint: None,
                    priority: 0,
                    limit_bps: None,
                    meta: {
                        let mut meta = std::collections::BTreeMap::from([(
                            "plugin".to_string(),
                            self.manifest.id.clone(),
                        )]);

                        /*
                         * The subscription's key, so the queue can find the
                         * item's own artwork in the local library rather than
                         * asking the network for a picture per row. It is the
                         * same `kind:itemId` a sandbox mod is keyed by.
                         */
                        if let Some(id) = ctx.get("id") {
                            meta.insert("item".to_string(), id.clone());
                        }

                        if let Some(kind) = ctx.get("kind") {
                            meta.insert("kind".to_string(), kind.clone());
                        }

                        meta
                    },
                })
                .await;
        }

        ensure_parent(target)?;

        match crate::download::fetch_direct(self.http, parsed, target, cap, expect_sha).await {
            Ok(()) => Ok(()),
            Err(crate::download::DirectFailure::Other(e)) => Err(e),
            Err(crate::download::DirectFailure::Checksum { actual }) => {
                audit!(
                    self.audit,
                    Error,
                    Install,
                    "plugin.download.checksum",
                    format!("{url}: expected {}, got {actual}", expect_sha.unwrap_or("")),
                    plugin = self.manifest.id
                );

                Err(AppError::jail(CHECKSUM_MISMATCH))
            }
        }
    }

    // ------------------------------------------------------------ JSON patch

    fn patch_json(&self, target: &Path, pointer: &str, value: &serde_json::Value) -> AppResult<()> {
        if !pointer.starts_with('/') || pointer.len() > 256 {
            return Err(AppError::invalid(
                "JSON pointer must start with '/' and be short.",
            ));
        }

        let mut doc: serde_json::Value = if target.exists() {
            let meta = std::fs::metadata(target)?;

            if meta.len() > MAX_JSON_BYTES {
                return Err(AppError::jail("Config file is too large to patch."));
            }

            serde_json::from_str(&std::fs::read_to_string(target)?)
                .map_err(|e| AppError::invalid(format!("Config file is not valid JSON: {e}")))?
        } else {
            serde_json::Value::Object(Default::default())
        };

        // Walk the pointer, creating intermediate objects. `serde_json`'s own
        // `pointer_mut` refuses to create missing nodes, which is exactly the
        // case a first-run config patch is in.
        let mut cursor = &mut doc;

        let parts: Vec<String> = pointer[1..]
            .split('/')
            .map(|p| p.replace("~1", "/").replace("~0", "~"))
            .collect();

        let Some((last, parents)) = parts.split_last() else {
            return Err(AppError::invalid("Empty JSON pointer."));
        };

        for part in parents {
            if !cursor.is_object() {
                *cursor = serde_json::Value::Object(Default::default());
            }

            cursor = cursor
                .as_object_mut()
                .expect("just ensured object")
                .entry(part.clone())
                .or_insert_with(|| serde_json::Value::Object(Default::default()));
        }

        if !cursor.is_object() {
            *cursor = serde_json::Value::Object(Default::default());
        }

        cursor
            .as_object_mut()
            .expect("just ensured object")
            .insert(last.clone(), value.clone());

        ensure_parent(target)?;

        // Temp-and-rename: a config the game reads must never be observed
        // half-written.
        let tmp = target.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&doc)?)?;
        std::fs::rename(&tmp, target)?;

        Ok(())
    }
}

// ------------------------------------------------------------------ Extract
//
// Free functions rather than methods on [`Executor`], because unpacking an
// archive is also what happens to a file the USER dropped into the window —
// see [`crate::local::store`]. Having one implementation of the zip-slip
// guard, the expansion cap and tar's link refusal is the whole reason: two
// would eventually differ, and the one that differed would be the one nobody
// was reading.

/// What to keep of an archive's paths. `Default` keeps everything as named.
#[derive(Default, Clone, Copy)]
pub(crate) struct ExtractOpts<'a> {
    /// Leading components to drop from every entry.
    pub strip: u8,
    /// After `strip`, drop one more component if every file in the archive
    /// shares one top-level folder. `__MACOSX/` is ignored and never written.
    pub strip_single_root: bool,
    pub include: &'a [String],
}

pub(crate) fn extract_archive(
    source: &Path,
    dest: &Path,
    opts: &ExtractOpts,
) -> AppResult<Vec<PathBuf>> {
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if name.ends_with(".zip") || name.ends_with(".mrpack") {
        extract_zip_within(source, dest, opts, MAX_EXTRACT_BYTES)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let open = || -> AppResult<_> {
            Ok(tar::Archive::new(flate2::read::GzDecoder::new(
                std::fs::File::open(source)?,
            )))
        };

        extract_tar(open, dest, opts)
    } else if name.ends_with(".tar") {
        let open = || -> AppResult<_> { Ok(tar::Archive::new(std::fs::File::open(source)?)) };

        extract_tar(open, dest, opts)
    } else if name.ends_with(".7z") {
        extract_7z_within(source, dest, opts, MAX_EXTRACT_BYTES)
    } else {
        Err(AppError::invalid(
            "Only .zip, .7z, .tar, .tar.gz and .tgz archives can be extracted.",
        ))
    }
}

/// The files written, in archive order — the caller journals them.
fn extract_zip_within(
    source: &Path,
    dest: &Path,
    opts: &ExtractOpts,
    max_bytes: u64,
) -> AppResult<Vec<PathBuf>> {
    let file = std::fs::File::open(source)?;

    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::invalid(format!("Not a readable zip: {e}")))?;

    if archive.len() > MAX_EXTRACT_ENTRIES {
        return Err(AppError::jail("Archive has too many entries."));
    }

    /*
     * One pass over the names alone (no decompression) decides whether there
     * is a single wrapper folder. The unsafe-path refusal is the same one the
     * extraction loop makes, so a hostile name fails here before any write.
     */
    let mut extra = 0usize;

    if opts.strip_single_root {
        let mut names: Vec<Vec<String>> = Vec::new();

        for i in 0..archive.len() {
            let entry = archive
                .by_index_raw(i)
                .map_err(|e| AppError::invalid(format!("Bad zip entry: {e}")))?;

            let Some(raw) = entry.enclosed_name() else {
                return Err(AppError::jail("Archive contains an unsafe path."));
            };

            if !entry.is_dir() {
                names.push(components(&raw));
            }
        }

        extra = single_root_extra(&names, opts.strip);
    }

    let mut total: u64 = 0;
    let mut written: Vec<PathBuf> = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::invalid(format!("Bad zip entry: {e}")))?;

        /*
         * `enclosed_name` is zip-rs's own zip-slip guard: it returns None
         * for absolute paths, `..` components and Windows drive prefixes.
         * We use it AND re-check through the jail, because it protects
         * against the archive's own claims while the jail additionally
         * protects against symlinks already on disk.
         */
        let Some(raw) = entry.enclosed_name() else {
            return Err(AppError::jail("Archive contains an unsafe path."));
        };

        if entry.is_dir() {
            continue;
        }

        if opts.strip_single_root && is_mac_junk(&raw) {
            continue;
        }

        let Some(relative) = strip_and_filter(&raw, opts.strip as usize + extra, opts.include)
        else {
            continue;
        };

        let too_large = || AppError::jail("Archive expands to more than the allowed size.");
        let remaining = max_bytes.saturating_sub(total);

        // The declared size is a cheap early refusal, and nothing more.
        if entry.size() > remaining {
            return Err(too_large());
        }

        let target = crate::plugins::jail::join_relative(dest, &relative)?;

        if !target.starts_with(dest) {
            return Err(AppError::jail("Archive entry escapes its destination."));
        }

        ensure_parent(&target)?;

        /*
         * The cap counts bytes actually WRITTEN. `entry.size()` is the
         * archive's own claim, and zip-rs does not hold the decompressor to
         * it — a deflate entry declaring one byte inflates to whatever it
         * inflates to, and the CRC mismatch surfaces only after every byte is
         * on disk. So the copy is bounded by what is left of the budget.
         */
        let mut out = std::fs::File::create(&target)?;
        let copied = std::io::copy(&mut (&mut entry).take(remaining + 1), &mut out)?;

        if copied > remaining {
            drop(out);
            let _ = std::fs::remove_file(&target);

            return Err(too_large());
        }

        total += copied;

        written.push(target);
    }

    Ok(written)
}

/// `open` yields a fresh reader each call: a tar stream cannot seek, so the
/// single-root scan and the extraction are two separate reads.
/// 7z, through the same caps and the same jail as the other formats.
///
/// What is refused beyond zip's list: a symlink or reparse-point entry (the 7z
/// header carries the Unix mode in the attribute word's high half, and a link
/// is a jail escape by construction), encrypted archives (no password is ever
/// supplied, so the reader fails), and a header whose declared sizes already
/// exceed the budget. As with zip, the declared size is only an early refusal:
/// the copy itself is bounded by what is left of the budget.
fn extract_7z_within(
    source: &Path,
    dest: &Path,
    opts: &ExtractOpts,
    max_bytes: u64,
) -> AppResult<Vec<PathBuf>> {
    use sevenz_rust2::{Password, SevenZReader};

    const UNIX_EXTENSION: u32 = 0x8000;
    const S_IFMT: u32 = 0o170_000;
    const S_IFLNK: u32 = 0o120_000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    let bad = |e: sevenz_rust2::Error| AppError::invalid(format!("Not a readable 7z: {e}"));
    let too_large = || AppError::jail("Archive expands to more than the allowed size.");

    let file = std::fs::File::open(source)?;
    let mut reader = SevenZReader::new(file, Password::empty()).map_err(bad)?;

    /*
     * The whole header is judged before any data is decoded: the entry count,
     * every name, every link attribute and the declared total. A solid block
     * cannot be skipped into, so refusing late would mean decoding first.
     */
    let files = &reader.archive().files;

    if files.len() > MAX_EXTRACT_ENTRIES {
        return Err(AppError::jail("Archive has too many entries."));
    }

    let mut declared: u64 = 0;
    let mut names: Vec<Vec<String>> = Vec::new();

    for entry in files {
        let raw = sevenz_name(entry.name())?;

        let attrs = entry.windows_attributes();
        let unix_mode = if attrs & UNIX_EXTENSION != 0 {
            attrs >> 16
        } else {
            0
        };

        if unix_mode & S_IFMT == S_IFLNK || attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(AppError::jail("Archive contains a link or special file."));
        }

        if entry.is_directory() || entry.is_anti_item() {
            continue;
        }

        declared = declared.saturating_add(entry.size());
        names.push(components(&raw));
    }

    if declared > max_bytes {
        return Err(too_large());
    }

    let extra = if opts.strip_single_root {
        single_root_extra(&names, opts.strip)
    } else {
        0
    };

    let mut total: u64 = 0;
    let mut written: Vec<PathBuf> = Vec::new();

    reader
        .for_each_entries(|entry, data| {
            // Errors from our own checks travel out of the library's closure as
            // io errors carrying a message, and are mapped back below.
            let refuse = |m: &str| sevenz_rust2::Error::from(std::io::Error::other(m.to_string()));

            if entry.is_directory() || entry.is_anti_item() {
                return Ok(true);
            }

            let raw = sevenz_name(entry.name()).map_err(|_| refuse("unsafe path"))?;

            if opts.strip_single_root && is_mac_junk(&raw) {
                return Ok(true);
            }

            let Some(relative) = strip_and_filter(&raw, opts.strip as usize + extra, opts.include)
            else {
                return Ok(true);
            };

            let remaining = max_bytes.saturating_sub(total);
            let target = crate::plugins::jail::join_relative(dest, &relative)
                .map_err(|_| refuse("unsafe path"))?;

            if !target.starts_with(dest) {
                return Err(refuse("unsafe path"));
            }

            ensure_parent(&target).map_err(|e| refuse(&e.to_string()))?;

            let mut out = std::fs::File::create(&target)?;
            let copied = std::io::copy(&mut data.take(remaining + 1), &mut out)?;

            if copied > remaining {
                drop(out);
                let _ = std::fs::remove_file(&target);

                return Err(refuse("too large"));
            }

            total += copied;
            written.push(target);

            Ok(true)
        })
        .map_err(|e| match e.to_string() {
            m if m.contains("too large") => too_large(),
            m if m.contains("unsafe path") => AppError::jail("Archive contains an unsafe path."),
            _ => bad(e),
        })?;

    Ok(written)
}

/// A 7z entry name as a relative path. `\` is a separator on the machine that
/// wrote it; anything that is not a plain relative component is refused.
fn sevenz_name(name: &str) -> AppResult<PathBuf> {
    let unified = name.replace('\\', "/");
    let path = PathBuf::from(&unified);

    let plain = path.components().all(|c| {
        matches!(
            c,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    });

    if unified.is_empty() || unified.starts_with('/') || unified.contains('\0') || !plain {
        return Err(AppError::jail("Archive contains an unsafe path."));
    }

    Ok(path)
}

fn extract_tar<R: Read>(
    open: impl Fn() -> AppResult<tar::Archive<R>>,
    dest: &Path,
    opts: &ExtractOpts,
) -> AppResult<Vec<PathBuf>> {
    let mut extra = 0usize;

    if opts.strip_single_root {
        let mut names: Vec<Vec<String>> = Vec::new();
        let mut scan = open()?;

        for entry in scan.entries()? {
            let entry = entry?;

            if entry.header().entry_type().is_file() {
                names.push(components(&entry.path()?));
            }

            if names.len() > MAX_EXTRACT_ENTRIES {
                return Err(AppError::jail("Archive has too many entries."));
            }
        }

        extra = single_root_extra(&names, opts.strip);
    }

    let mut archive = open()?;
    let mut total: u64 = 0;
    let mut written: Vec<PathBuf> = Vec::new();

    for entry in archive.entries()? {
        let mut entry = entry?;

        /*
         * Tar can carry symlinks and hardlinks, and both are a jail escape
         * by construction: a `link → /etc` entry followed by a `link/passwd`
         * entry writes outside any destination check that looks only at
         * paths. Nothing a mod archive needs requires them.
         */
        let kind = entry.header().entry_type();

        if !kind.is_file() && !kind.is_dir() {
            return Err(AppError::jail("Archive contains a link or special file."));
        }

        if kind.is_dir() {
            continue;
        }

        let raw = entry.path()?.into_owned();

        if opts.strip_single_root && is_mac_junk(&raw) {
            continue;
        }

        let Some(relative) = strip_and_filter(&raw, opts.strip as usize + extra, opts.include)
        else {
            continue;
        };

        total += entry.size();

        if total > MAX_EXTRACT_BYTES {
            return Err(AppError::jail(
                "Archive expands to more than the allowed size.",
            ));
        }

        if written.len() >= MAX_EXTRACT_ENTRIES {
            return Err(AppError::jail("Archive has too many entries."));
        }

        let target = crate::plugins::jail::join_relative(dest, &relative)?;

        if !target.starts_with(dest) {
            return Err(AppError::jail("Archive entry escapes its destination."));
        }

        ensure_parent(&target)?;

        let mut out = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;

        written.push(target);
    }

    Ok(written)
}

fn components(raw: &Path) -> Vec<String> {
    raw.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(p) => Some(p.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// The `__MACOSX/` resource-fork sibling Finder adds to a zip.
fn is_mac_junk(raw: &Path) -> bool {
    components(raw).first().is_some_and(|c| c == "__MACOSX")
}

/// 1 when, after `strip`, every file lives under one shared top-level folder
/// (and none sits directly at the top), else 0. An archive with no files, or
/// whose files already sit at the root, is left alone.
fn single_root_extra(files: &[Vec<String>], strip: u8) -> usize {
    let strip = strip as usize;
    let mut root: Option<&str> = None;

    for parts in files {
        if parts.first().is_some_and(|c| c == "__MACOSX") {
            continue;
        }

        // A file that `strip` consumes entirely is not extracted anyway.
        if parts.len() <= strip {
            continue;
        }

        let rest = &parts[strip..];

        if rest.len() < 2 {
            return 0;
        }

        match root {
            None => root = Some(rest[0].as_str()),
            Some(r) if r == rest[0] => {}
            Some(_) => return 0,
        }
    }

    usize::from(root.is_some())
}

/// Apply `strip` and the include filter; `None` means "skip this entry".
fn strip_and_filter(raw: &Path, strip: usize, include: &[String]) -> Option<String> {
    let parts = components(raw);

    if parts.len() <= strip {
        return None;
    }

    let relative = parts[strip..].join("/");

    if relative.is_empty() {
        return None;
    }

    if !include.is_empty() && !include.iter().any(|p| relative.starts_with(p.as_str())) {
        return None;
    }

    Some(relative)
}

pub(crate) fn ensure_parent(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    Ok(())
}

/// Paths in the log are shown as-is. They are the user's own directories and
/// the whole value of the log is being able to see exactly what was touched.
pub(crate) fn display(path: &Path) -> String {
    path.display().to_string()
}

pub(crate) fn step_label(step: &Step) -> String {
    match step {
        Step::Download { url, artifact, .. } => match artifact {
            Some(key) => format!("download artifact {key}"),
            None => format!("download {url}"),
        },
        Step::Extract { from, .. } => format!("extract {}", from.path),
        Step::Copy { from, to } => format!("copy {} → {}", from.path, to.path),
        Step::Move { from, to } => format!("move {} → {}", from.path, to.path),
        Step::Mkdir { path } => format!("mkdir {}", path.path),
        Step::Remove { path } => format!("remove {}", path.path),
        Step::WriteText { path, .. } => format!("write {}", path.path),
        Step::PatchJson { path, pointer, .. } => format!("patch {}{}", path.path, pointer),
        Step::RunInstaller { program, .. } => format!("run installer {program}"),
        Step::DownloadList { list, .. } => format!("download the list in {}", list.path),
        Step::MergeJson { base, overlay, to, .. } => {
            format!("merge {} + {} → {}", base.path, overlay.path, to.path)
        }
        Step::RequireFile { path, .. } => format!("require {}", path.path),
    }
}

/// Which jail roots a plan needs, so the UI can tell the user up front
/// rather than failing halfway through.
pub fn required_roots(steps: &[Step]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();

    let mut note = |p: &PathRef| {
        let key = p.root.name();

        if !out.contains(&key) {
            out.push(key);
        }
    };

    for step in steps {
        for path in step.paths() {
            note(path);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_release_file_download_is_checked_against_the_servers_digest() {
        let ctx = RunContext(HashMap::from([
            ("fileUrl".to_string(), "https://x/f".to_string()),
            ("fileSha256".to_string(), "ab12".to_string()),
        ]));

        assert_eq!(
            ctx.expected_sha("https://x/f", None).as_deref(),
            Some("ab12")
        );
        assert_eq!(ctx.expected_sha("https://x/other", None), None);
        assert_eq!(
            ctx.expected_sha("https://x/f", Some("cd")).as_deref(),
            Some("cd")
        );
    }

    use super::*;

    use std::path::PathBuf;

    #[test]
    fn strip_drops_leading_components() {
        let out = strip_and_filter(Path::new("pkg-1.0/mods/a.jar"), 1, &[]);
        assert_eq!(out.as_deref(), Some("mods/a.jar"));
    }

    #[test]
    fn strip_past_the_end_skips_the_entry() {
        assert!(strip_and_filter(Path::new("a.jar"), 2, &[]).is_none());
    }

    #[test]
    fn include_filters_by_prefix() {
        let path = Path::new("root/mods/a.jar");

        assert!(strip_and_filter(path, 1, &["mods".into()]).is_some());
        assert!(strip_and_filter(path, 1, &["config".into()]).is_none());
    }

    #[test]
    fn context_substitution_is_literal() {
        let ctx = RunContext(HashMap::from([("id".into(), "42".into())]));

        assert_eq!(ctx.fill("https://x.test/{id}.zip"), "https://x.test/42.zip");
        // No expression evaluation, no recursion into the substituted value.
        assert_eq!(ctx.fill("{unknown}"), "{unknown}");
    }

    // ------------------------------------------------- Placeholders in paths

    /// A manifest that may write anywhere under the game folder.
    fn writable_manifest() -> Manifest {
        Manifest {
            manifest_version: 1,
            id: "app.test".into(),
            name: "t".into(),
            version: "1".into(),
            author: "t".into(),
            description: None,
            homepage: None,
            apps: vec![],
            permissions: crate::plugins::manifest::Permissions {
                fs: vec![crate::plugins::manifest::FsGrant {
                    root: crate::plugins::manifest::FsRoot::GameDir,
                    path: String::new(),
                    write: true,
                }],
                ..Default::default()
            },
            installer: None,
            server_query: None,
            theme: None,
            manager: None,
        }
    }

    fn jail_over(manifest: &Manifest, game: &Path) -> Jail {
        let mut available: HashMap<&'static str, PathBuf> = HashMap::new();

        available.insert("gameDir", game.to_path_buf());

        Jail::build(manifest, &available).expect("jail")
    }

    /// Write one file through the executor, and say whether it worked.
    async fn write_through(game: &Path, audit_at: &Path, template: &str, ctx: &RunContext) -> bool {
        let manifest = writable_manifest();
        let jail = jail_over(&manifest, game);
        let audit = Audit::new(audit_at.to_path_buf());

        let executor = Executor {
            manifest: &manifest,
            jail: &jail,
            http: &crate::http::client(),
            audit: &audit,
            downloads: None,
        };

        let steps = vec![Step::WriteText {
            path: PathRef {
                root: crate::plugins::manifest::FsRoot::GameDir,
                path: template.into(),
            },
            content: "x".into(),
            if_missing: false,
        }];

        executor.run(&steps, ctx).await.ok
    }

    /// The journal names every extracted FILE, never the folder it went into.
    ///
    /// The regression this pins was a data-loss bug with a comment pointing
    /// straight at it. `library::install::run_uninstall` sweeps the journal by
    /// removing each recorded path, and a directory goes through
    /// `remove_dir_all` — so an extract that recorded only its destination
    /// meant uninstalling one mod deleted the shared folder every other mod had
    /// extracted into. The shipped GTA V rule says in its own uninstall comment
    /// that it relies on the journal for exactly this, and `gameDir/scripts` is
    /// precisely such a shared folder.
    /// A zip entry whose central directory claims one byte but whose data is
    /// 64 KiB. The byte cap must count what is written, not what the archive
    /// says, or a lying header walks straight past the zip-bomb limit.
    #[test]
    fn a_zip_entry_lying_about_its_size_cannot_pass_the_byte_cap() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let archive = tmp.path().join("bomb.zip");
        let body = vec![b'A'; 64 * 1024];

        let mut bytes = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
            let opts: zip::write::FileOptions<'_, '_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);

            zip.start_file("big.bin", opts).expect("entry");
            std::io::Write::write_all(&mut zip, &body).expect("body");
            zip.finish().expect("finish");
        }

        // Rewrite the uncompressed size in both the local and central headers.
        let patch = |bytes: &mut Vec<u8>, sig: &[u8; 4], offset: usize| {
            let at = bytes
                .windows(4)
                .position(|w| w == sig)
                .expect("header present");
            bytes[at + offset..at + offset + 4].copy_from_slice(&1u32.to_le_bytes());
        };
        patch(&mut bytes, b"PK\x03\x04", 22);
        patch(&mut bytes, b"PK\x01\x02", 24);

        std::fs::write(&archive, &bytes).expect("write archive");

        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).expect("mkdir");

        let result = extract_zip_within(&archive, &dest, &ExtractOpts::default(), 1024);

        // Refused either by our own byte cap ("allowed size") or, since zip 9, by
        // the reader itself noticing the header and the data disagree. Either
        // way the lie must not be extracted whole.
        result.expect_err("a lying entry was extracted whole");
        assert!(
            std::fs::metadata(dest.join("big.bin")).map_or(0, |m| m.len()) <= 1024,
            "more than the cap reached the disk"
        );
    }

    fn zip_of(path: &Path, names: &[&str]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).expect("create"));
        let opts: zip::write::FileOptions<'_, '_, ()> = zip::write::FileOptions::default();

        for name in names {
            zip.start_file(*name, opts).expect("entry");
            std::io::Write::write_all(&mut zip, b"data").expect("body");
        }
        zip.finish().expect("finish");
    }

    fn extract_names(names: &[&str], opts: &ExtractOpts) -> Vec<String> {
        let tmp = tempfile::tempdir().expect("tempdir");
        let archive = tmp.path().join("m.zip");
        zip_of(&archive, names);
        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).expect("mkdir");

        let mut got: Vec<String> = extract_archive(&archive, &dest, opts)
            .expect("extract")
            .iter()
            .map(|p| {
                p.strip_prefix(&dest)
                    .expect("inside")
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        got.sort();
        got
    }

    const SINGLE: ExtractOpts<'static> = ExtractOpts {
        strip: 0,
        strip_single_root: true,
        include: &[],
    };

    #[test]
    fn a_single_wrapper_folder_is_stripped_and_mac_junk_dropped() {
        let got = extract_names(
            &[
                "mod-1.2/descriptor.mod",
                "mod-1.2/common/a.txt",
                "__MACOSX/mod-1.2/._a",
            ],
            &SINGLE,
        );

        assert_eq!(got, ["common/a.txt", "descriptor.mod"]);
    }

    #[test]
    fn no_wrapper_or_several_roots_are_left_alone() {
        // Files at the top: nothing to unwrap.
        assert_eq!(
            extract_names(&["descriptor.mod", "common/a.txt"], &SINGLE),
            ["common/a.txt", "descriptor.mod"]
        );
        // Two top-level folders: not a wrapper.
        assert_eq!(
            extract_names(&["a/x.txt", "b/y.txt"], &SINGLE),
            ["a/x.txt", "b/y.txt"]
        );
        // One folder plus a loose file: not a wrapper.
        assert_eq!(
            extract_names(&["a/x.txt", "readme.txt"], &SINGLE),
            ["a/x.txt", "readme.txt"]
        );
        // Without the option, a single folder is kept.
        assert_eq!(
            extract_names(&["a/x.txt"], &ExtractOpts::default()),
            ["a/x.txt"]
        );
    }

    #[test]
    fn single_root_adds_to_numeric_strip() {
        let opts = ExtractOpts { strip: 1, ..SINGLE };

        assert_eq!(
            extract_names(&["outer/inner/x.txt", "outer/inner/y/z.txt"], &opts),
            ["x.txt", "y/z.txt"]
        );
    }

    #[test]
    fn single_root_keeps_the_zip_slip_refusal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let archive = tmp.path().join("evil.zip");
        zip_of(&archive, &["w/ok.txt", "w/../../escape.txt"]);
        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).expect("mkdir");

        extract_archive(&archive, &dest, &SINGLE).expect_err("unsafe path accepted");
        assert!(!tmp.path().join("escape.txt").exists());
    }

    #[test]
    fn single_root_keeps_the_byte_cap() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let archive = tmp.path().join("big.zip");
        zip_of(&archive, &["w/a.txt", "w/b.txt"]);
        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).expect("mkdir");

        // Each entry is 4 bytes; a cap of 6 lets one through and refuses the second.
        extract_zip_within(&archive, &dest, &SINGLE, 6).expect_err("cap ignored");
    }

    #[test]
    fn a_tar_gz_wrapper_folder_is_stripped() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let archive = tmp.path().join("m.tar.gz");
        {
            let gz = flate2::write::GzEncoder::new(
                std::fs::File::create(&archive).expect("create"),
                flate2::Compression::default(),
            );
            let mut tar = tar::Builder::new(gz);

            for name in ["pkg/a.txt", "pkg/sub/b.txt"] {
                let mut h = tar::Header::new_gnu();
                h.set_size(4);
                h.set_mode(0o644);
                h.set_cksum();
                tar.append_data(&mut h, name, &b"data"[..]).expect("append");
            }
            tar.into_inner().expect("tar").finish().expect("gz");
        }
        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).expect("mkdir");

        extract_archive(&archive, &dest, &SINGLE).expect("extract");

        assert!(dest.join("a.txt").is_file());
        assert!(dest.join("sub/b.txt").is_file());
        assert!(!dest.join("pkg").exists());
    }

    /// `(name, body, windows_attributes)` entries into a fresh `.7z`.
    fn sevenz_of(path: &Path, entries: &[(&str, &[u8], u32)]) {
        use sevenz_rust2::{SevenZArchiveEntry, SevenZWriter};

        let mut w = SevenZWriter::create(path).expect("create 7z");

        for (name, body, attrs) in entries {
            let mut e = SevenZArchiveEntry::new_file(name);
            if *attrs != 0 {
                e.has_windows_attributes = true;
                e.windows_attributes = *attrs;
            }
            w.push_archive_entry(e, Some(*body)).expect("push");
        }
        w.finish().expect("finish");
    }

    fn sevenz_extract(
        entries: &[(&str, &[u8], u32)],
        opts: &ExtractOpts,
        cap: u64,
    ) -> (tempfile::TempDir, AppResult<Vec<PathBuf>>) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let archive = tmp.path().join("m.7z");
        sevenz_of(&archive, entries);
        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).expect("mkdir");

        let r = extract_7z_within(&archive, &dest, opts, cap);
        (tmp, r)
    }

    #[test]
    fn a_7z_extracts_and_honours_strip_single_root() {
        let (tmp, r) = sevenz_extract(
            &[("w/a.txt", b"one", 0), ("w/sub/b.txt", b"two", 0)],
            &SINGLE,
            1024,
        );

        assert_eq!(r.expect("extract").len(), 2);
        assert_eq!(
            std::fs::read(tmp.path().join("out/a.txt")).expect("a"),
            b"one"
        );
        assert!(tmp.path().join("out/sub/b.txt").is_file());
        assert!(!tmp.path().join("out/w").exists());
    }

    #[test]
    fn a_7z_extract_is_dispatched_by_extension() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let archive = tmp.path().join("M.7Z");
        sevenz_of(&archive, &[("x.txt", b"x", 0)]);
        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).expect("mkdir");

        extract_archive(&archive, &dest, &ExtractOpts::default()).expect("extract");
        assert!(dest.join("x.txt").is_file());
    }

    #[test]
    fn a_7z_path_escape_is_refused_and_nothing_is_written_outside() {
        for name in [
            "../escape.txt",
            "a/../../escape.txt",
            "/abs.txt",
            "..\\escape.txt",
        ] {
            let (tmp, r) = sevenz_extract(&[(name, b"x", 0)], &ExtractOpts::default(), 1024);

            r.expect_err(name);
            assert!(!tmp.path().join("escape.txt").exists(), "{name} escaped");
            assert!(!tmp.path().join("abs.txt").exists());
        }
    }

    #[test]
    fn a_7z_symlink_entry_is_refused() {
        // Unix extension bit plus S_IFLNK in the high half.
        let attrs = 0x8000 | (0o120_777u32 << 16);
        let (_tmp, r) = sevenz_extract(&[("link", b"/etc", attrs)], &ExtractOpts::default(), 1024);

        r.expect_err("symlink accepted");
    }

    #[test]
    fn a_7z_over_the_byte_cap_is_refused() {
        // Declared total over the cap: refused before any data is decoded.
        let (tmp, r) = sevenz_extract(
            &[("a.bin", &[7u8; 600], 0), ("b.bin", &[7u8; 600], 0)],
            &ExtractOpts::default(),
            1000,
        );

        r.expect_err("cap ignored");
        assert!(!tmp.path().join("out/a.bin").exists());
    }

    #[test]
    fn a_7z_that_is_not_a_7z_is_an_error_not_a_panic() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let archive = tmp.path().join("junk.7z");
        std::fs::write(&archive, b"7z\xbc\xaf\x27\x1c\x00\x04 truncated nonsense").expect("write");
        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).expect("mkdir");

        extract_archive(&archive, &dest, &ExtractOpts::default()).expect_err("junk accepted");
    }

    #[test]
    fn a_zip_within_the_byte_cap_still_extracts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let archive = tmp.path().join("ok.zip");
        {
            let file = std::fs::File::create(&archive).expect("create");
            let mut zip = zip::ZipWriter::new(file);
            let opts: zip::write::FileOptions<'_, '_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);

            for name in ["a.txt", "b.txt"] {
                zip.start_file(name, opts).expect("entry");
                std::io::Write::write_all(&mut zip, &[b'x'; 512]).expect("body");
            }
            zip.finish().expect("finish");
        }

        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).expect("mkdir");

        // Exactly at the cap is allowed.
        let written =
            extract_zip_within(&archive, &dest, &ExtractOpts::default(), 1024).expect("extract");

        assert_eq!(written.len(), 2);
        assert_eq!(std::fs::read(dest.join("b.txt")).expect("read").len(), 512);
    }

    #[tokio::test]
    async fn extracting_journals_the_files_and_not_the_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");
        std::fs::create_dir_all(game.join("scripts")).expect("mkdir");

        // A file another mod already put there. It must survive.
        std::fs::write(game.join("scripts/other.lua"), "-- someone else's\n").expect("write");

        let archive = tmp.path().join("mod.zip");
        {
            let file = std::fs::File::create(&archive).expect("create");
            let mut zip = zip::ZipWriter::new(file);
            let opts: zip::write::FileOptions<'_, '_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);

            for name in ["a.lua", "nested/b.lua"] {
                zip.start_file(name, opts).expect("entry");
                std::io::Write::write_all(&mut zip, b"body").expect("body");
            }

            zip.finish().expect("finish");
        }

        std::fs::copy(&archive, game.join("mod.zip")).expect("stage the archive");

        let manifest = writable_manifest();
        let jail = jail_over(&manifest, &game);
        let audit = Audit::new(tmp.path().join("audit.jsonl"));

        let executor = Executor {
            manifest: &manifest,
            jail: &jail,
            http: &crate::http::client(),
            audit: &audit,
            downloads: None,
        };

        let gd = crate::plugins::manifest::FsRoot::GameDir;

        let report = executor
            .run(
                &[Step::Extract {
                    from: PathRef {
                        root: gd,
                        path: "mod.zip".into(),
                    },
                    to: PathRef {
                        root: gd,
                        path: "scripts".into(),
                    },
                    strip: 0,
                    strip_single_root: false,
                    include: vec![],
                }],
                &RunContext(HashMap::new()),
            )
            .await;

        assert!(report.ok, "extract failed: {:?}", report.error);

        let journal = &report.applied;

        assert!(
            journal.iter().any(|p| p.ends_with("a.lua"))
                && journal.iter().any(|p| p.ends_with("b.lua")),
            "the journal should name every extracted file: {journal:?}"
        );
        assert!(
            !journal.iter().any(|p| p.ends_with("scripts")),
            "the journal must not name the destination directory: {journal:?}"
        );
        assert!(
            !journal.iter().any(|p| p.ends_with("other.lua")),
            "only what this install wrote belongs in its journal: {journal:?}"
        );
        assert!(
            game.join("scripts/other.lua").exists(),
            "the other mod's file was disturbed"
        );
    }

    /// A placeholder in a PATH is filled, exactly as one in a URL is.
    ///
    /// The regression: they were not. `mods/{fileName}` produced a file
    /// literally called `{fileName}`, so every mod for a game wrote to the same
    /// one — and the shipped example rules all use that spelling. It survived
    /// because a path with braces in it is a perfectly valid path, so nothing
    /// failed until a mod was actually installed.
    #[tokio::test]
    async fn a_placeholder_in_a_path_is_filled() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");

        std::fs::create_dir_all(&game).expect("mkdir");

        let ctx = RunContext(HashMap::from([("fileName".into(), "cool.jar".into())]));

        assert!(
            write_through(
                &game,
                &tmp.path().join("audit.jsonl"),
                "mods/{fileName}",
                &ctx
            )
            .await
        );

        assert!(game.join("mods/cool.jar").is_file(), "the filled name");
        assert!(
            !game.join("mods/{fileName}").exists(),
            "and not the literal one"
        );
    }

    /// Filling happens BEFORE the jail resolves, so a hostile value is refused
    /// by the same check that refuses a hostile literal.
    ///
    /// Item names are author-written text off the network. Were substitution to
    /// run after the containment check, this would be a plain traversal.
    #[tokio::test]
    async fn a_placeholder_cannot_carry_a_path_out_of_the_jail() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");

        std::fs::create_dir_all(&game).expect("mkdir");

        let cases = [
            // Traversal out of the subdirectory the template names.
            ("mods/{itemName}", "../../escaped.txt"),
            ("mods/{itemName}", "../../../escaped.txt"),
            // A template that is NOTHING but a placeholder, where an absolute
            // value really is absolute — `Path::join` would discard the base.
            ("{itemName}", "/etc/escaped"),
            ("{itemName}", "C:\\Windows\\escaped"),
            ("{itemName}", "../escaped.txt"),
        ];

        for (template, hostile) in cases {
            let ctx = RunContext(HashMap::from([("itemName".into(), hostile.to_string())]));

            assert!(
                !write_through(&game, &tmp.path().join("audit.jsonl"), template, &ctx).await,
                "{template} filled with {hostile} should be refused"
            );
        }

        /*
         * NOT in the list, and worth saying why: `mods/{itemName}` filled with
         * `/etc/escaped` produces `mods//etc/escaped`, which is a RELATIVE path
         * with an empty component in it and resolves inside the jail. It is
         * allowed, and allowing it is correct — an absolute-looking value in
         * the middle of a template is not absolute.
         */

        assert!(
            !tmp.path().join("escaped.txt").exists(),
            "nothing landed outside"
        );
    }

    // ---------------------------------------------------------- mirror lists

    const PIN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn net_manifest(hosts: &[&str]) -> Manifest {
        let mut m = writable_manifest();

        m.permissions.net = hosts.iter().map(|h| h.to_string()).collect();
        m
    }

    /// Run `download_mirrored` offline and return its result and the audit text.
    async fn mirrored(urls: &[&str], pin: Option<&str>, hosts: &[&str]) -> (AppResult<()>, String) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");

        std::fs::create_dir_all(&game).expect("mkdir");

        let manifest = net_manifest(hosts);
        let jail = jail_over(&manifest, &game);
        let audit_at = tmp.path().join("audit.jsonl");
        let audit = Audit::new(audit_at.clone());

        let executor = Executor {
            manifest: &manifest,
            jail: &jail,
            http: &crate::http::client(),
            audit: &audit,
            downloads: None,
        };

        let urls: Vec<String> = urls.iter().map(|u| u.to_string()).collect();

        let result = executor
            .download_mirrored(
                &urls,
                &game.join("f.bin"),
                pin,
                Some(1024),
                &RunContext(HashMap::new()),
            )
            .await;

        drop(audit);

        let log = std::fs::read_to_string(&audit_at).unwrap_or_default();

        (result, log)
    }

    #[tokio::test]
    async fn mirrors_without_a_well_formed_pin_are_refused_before_any_request() {
        for pin in [None, Some(""), Some("abc"), Some(&PIN[..63])] {
            let (r, log) = mirrored(
                &["https://127.0.0.1:1/a", "https://127.0.0.1:2/b"],
                pin,
                &["127.0.0.1"],
            )
            .await;

            assert!(matches!(r, Err(AppError::Jail(_))), "{pin:?}");
            assert!(!log.contains("plugin.download"), "no request for {pin:?}");
        }
    }

    #[tokio::test]
    async fn every_mirror_passes_the_same_https_and_host_checks() {
        let bad = [
            "http://127.0.0.1:2/b",
            "https://evil.example/b",
            "ftp://127.0.0.1/b",
            "not a url",
        ];

        for mirror in bad {
            let (r, log) = mirrored(
                &["https://127.0.0.1:1/a", mirror],
                Some(PIN),
                &["127.0.0.1"],
            )
            .await;

            assert!(matches!(r, Err(AppError::Jail(_))), "{mirror}");

            // Refused up front: the good primary was never even tried.
            assert!(
                !log.contains("plugin.download"),
                "{mirror} reached the network"
            );
        }
    }

    /// Two unreachable mirrors: the second IS tried (a failover is audited),
    /// and the result is the network error, not a success and not a panic.
    #[tokio::test]
    async fn a_network_failure_moves_on_to_the_next_mirror() {
        let (r, log) = mirrored(
            &["https://127.0.0.1:1/a", "https://127.0.0.1:2/b"],
            Some(PIN),
            &["127.0.0.1"],
        )
        .await;

        assert!(r.is_err());
        assert!(log.contains("127.0.0.1:1/a"), "primary tried");
        assert!(log.contains("127.0.0.1:2/b"), "mirror tried");
        assert!(log.contains("plugin.download.failover"));
    }

    #[test]
    fn only_a_network_failure_or_a_mismatch_fails_over() {
        assert!(failover_worthy(&AppError::Network("down".into())));
        assert!(failover_worthy(&AppError::jail(CHECKSUM_MISMATCH)));

        assert!(!failover_worthy(&AppError::jail("not in allowed hosts")));
        assert!(!failover_worthy(&AppError::invalid(
            "The download was cancelled."
        )));
        assert!(!failover_worthy(&AppError::internal("x")));
    }

    #[test]
    fn a_manifest_with_mirrors_and_no_pin_does_not_load() {
        let manifest = |extra: &str| {
            format!(
                r#"{{"manifestVersion":1,"id":"app.test","name":"m","version":"1","author":"a",
                "permissions":{{"net":["a.test"],"fs":[{{"root":"gameDir","path":"","write":true}}]}},
                "installer":{{"install":[{{"action":"download","url":"https://a.test/f",
                "mirrors":["https://a.test/g"],"to":{{"root":"gameDir","path":"f"}}{extra}}}]}}}}"#
            )
        };

        assert!(Manifest::parse(&manifest("")).is_err());
        assert!(Manifest::parse(&manifest(r#","sha256":"abc""#)).is_err());
        let ok = Manifest::parse(&manifest(&format!(r#","sha256":"{PIN}""#)));

        assert!(ok.is_ok(), "{:?}", ok.err());
    }
}
