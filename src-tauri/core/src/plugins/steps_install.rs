//! The install-manifest steps: `runInstaller`, `downloadList`, `mergeJson`
//! and `requireFile`, and the run of a whole install manifest
//! ([`Executor::run_install`]). See `docs/loader-manifests.md`.
//!
//! None of these is reachable from a plugin. The parser refuses them
//! ([`crate::plugins::manifest::check_plugin_steps`]) and the dispatch in
//! [`Executor::run_step`] refuses them again without an [`InstallRun`].
//!
//! The process runner lives here rather than in `src-tauri/src/spawn.rs`
//! because the executor that calls it is in this crate and the app crate is
//! not a dependency of it. `spawn.rs::spawn_installer` is the app's entry to
//! the same function, so there is still exactly one way an installer starts.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::audit;
use crate::error::{AppError, AppResult};
use crate::layers::placeholders::Values;
use crate::layers::progress::StepStatus;
use crate::plugins::manifest::{
    FsGrant, FsRoot, InstallStep, Manifest, MergeMode, MergeRule, PathRef, Permissions, Step,
    WhenContext,
};
use crate::plugins::steps::{
    display, ensure_parent, is_sha256_hex, step_label, Executor, RunContext, RunReport,
};

/// `runInstaller.timeoutSecs` when the manifest does not set one.
pub const DEFAULT_INSTALLER_TIMEOUT_SECS: u64 = 900;

/// `downloadList.max` when the manifest does not set one.
pub const DEFAULT_LIST_MAX: u64 = 10_000;

/// Cap on a JSON document `downloadList` or `mergeJson` reads. A version
/// JSON is tens of KB and an asset index a few MB.
const MAX_DOC_BYTES: u64 = 64 * 1024 * 1024;

/// Never passed to an installer, even through a proxy setting: each one
/// changes what a JVM loads before the installer's own code runs.
pub const FORBIDDEN_ENV: [&str; 3] = ["JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "CLASSPATH"];

// ------------------------------------------------------------- Run inputs

/// The binding for one `artifacts.<key>`: the URL, pinned digest and size
/// the catalog resolved.
#[derive(Debug, Clone)]
pub struct ArtifactPin {
    pub url: String,
    pub sha256: String,
    pub mirrors: Vec<String>,
    pub size: Option<u64>,
}

/// One installed runtime, by the kind its placeholder names (`java`).
#[derive(Debug, Clone)]
pub struct RuntimeExe {
    /// The entry executable. Its folder is the installer's whole `PATH`.
    pub exe: PathBuf,
    /// A Java runtime: `JAVA_HOME` is set and the tmpdir and home flags are
    /// prepended to the arguments.
    pub java: bool,
}

/// A line of installer output, for the progress UI.
pub type OutputSink = Arc<dyn Fn(&str) + Send + Sync>;

/// Everything an install-manifest run needs that the manifest does not say.
#[derive(Clone, Default)]
pub struct InstallEnv {
    /// `gameVersion`, `platform`, `side`, `launchVersionId`, `var.*`, and for
    /// a loader `loader` and `loaderVersion`.
    pub values: Values,
    pub when: WhenContext,
    pub artifacts: HashMap<String, ArtifactPin>,
    pub runtimes: HashMap<String, RuntimeExe>,
    /// The app's proxy variables (`HTTPS_PROXY`, ...), when the user set one.
    pub proxy: Vec<(String, String)>,
    pub on_output: Option<OutputSink>,
    /// Each step's start, end, skip or failure, by its index in the steps run.
    pub on_step: Option<crate::layers::progress::StepSink>,
}

/// The state one install run carries from step to step.
pub(crate) struct InstallRun<'e> {
    pub(crate) env: &'e InstallEnv,
    /// Files a pinned `download` wrote in this run, with the digest they had.
    /// `downloadList` reads only these, re-hashing first, so a list replaced
    /// by a later `writeText` or `runInstaller` is refused.
    pub(crate) pinned: HashMap<PathBuf, String>,
    /// `{artifact.<key>}`: where each artifact download landed.
    pub(crate) artifacts: HashMap<String, PathBuf>,
}

/// The result of [`Executor::run_install`]: the plugin-style report plus the
/// steps whose `when` did not match, which are reported, never dropped.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    #[serde(flatten)]
    pub run: RunReport,
    pub skipped: Vec<usize>,
}

/// The synthetic manifest an install run's [`Executor`] and jail are built
/// from: the install manifest's `downloadHosts`, and the grants every install
/// manifest has (`layer` and `downloads` read-write, `runtime` read-only).
pub fn install_manifest(id: &str, title: &str, download_hosts: &[String]) -> Manifest {
    let grant = |root, write| FsGrant {
        root,
        path: String::new(),
        write,
    };

    Manifest {
        manifest_version: 1,
        id: id.to_string(),
        name: title.to_string(),
        version: "1".into(),
        author: "TMC".into(),
        description: None,
        homepage: None,
        apps: vec![],
        permissions: Permissions {
            fs: vec![
                grant(FsRoot::Layer, true),
                grant(FsRoot::Downloads, true),
                grant(FsRoot::Runtime, false),
            ],
            net: download_hosts.to_vec(),
            ..Default::default()
        },
        installer: None,
        server_query: None,
        theme: None,
        manager: None,
    }
}

/// The `available` map for [`crate::plugins::jail::Jail::build`]: the layer
/// being built (its `.partial` folder), the download cache and, when the
/// manifest needs one, the runtime folder.
pub fn install_roots(
    layer: &Path,
    downloads: &Path,
    runtime: Option<&Path>,
) -> HashMap<&'static str, PathBuf> {
    let mut roots = HashMap::from([
        (FsRoot::Layer.name(), layer.to_path_buf()),
        (FsRoot::Downloads.name(), downloads.to_path_buf()),
    ]);

    if let Some(runtime) = runtime {
        roots.insert(FsRoot::Runtime.name(), runtime.to_path_buf());
    }

    roots
}

// ------------------------------------------------------------ The run

impl Executor<'_> {
    /// Runs an install manifest's steps into the layer this executor's jail
    /// was built over ([`install_manifest`], [`install_roots`]).
    ///
    /// Placeholders are expanded per step before it runs, strictly: a token
    /// with no value fails the step instead of reaching a path. The
    /// installer-only placeholders (`{artifact.*}`, `{runtime.*}`,
    /// `{layerFiles}`, `{layerWork}`) are not in the general set, so they
    /// expand only where `runInstaller` expands them.
    pub async fn run_install(&self, steps: &[InstallStep], env: &InstallEnv) -> InstallReport {
        let mut run = InstallRun {
            env,
            pinned: HashMap::new(),
            artifacts: HashMap::new(),
        };
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        let total = steps.len();
        let report = |index: usize, status, log: Option<String>| {
            if let Some(sink) = &env.on_step {
                sink(index, status, log);
            }
        };

        for (index, item) in steps.iter().enumerate() {
            let label = item.label.clone().unwrap_or_else(|| step_label(&item.step));
            let at = format!("[{}/{total}] {label}", index + 1);

            if item.when.as_ref().is_some_and(|w| !w.matches(&env.when)) {
                audit!(
                    self.audit,
                    Info,
                    Install,
                    "install.step.skip",
                    at,
                    plugin = self.manifest.id
                );
                skipped.push(index);
                report(index, StepStatus::Skipped, None);
                continue;
            }

            audit!(
                self.audit,
                Info,
                Install,
                "install.step.start",
                at.clone(),
                plugin = self.manifest.id
            );
            report(index, StepStatus::Running, None);

            if let Err(err) = self.install_step(&item.step, &mut run, &mut applied).await {
                audit!(
                    self.audit,
                    Error,
                    Install,
                    "install.step.fail",
                    format!("{at}: {}", err.detail()),
                    plugin = self.manifest.id
                );
                report(index, StepStatus::Failed, Some(err.detail()));

                return InstallReport {
                    run: RunReport {
                        ok: false,
                        steps_total: total,
                        steps_run: index,
                        applied,
                        failed_at: Some(index),
                        error: Some(err.to_string()),
                    },
                    skipped,
                };
            }

            audit!(
                self.audit,
                Info,
                Install,
                "install.step.ok",
                at,
                plugin = self.manifest.id
            );
            report(index, StepStatus::Done, None);
        }

        InstallReport {
            run: RunReport {
                ok: true,
                steps_total: total,
                steps_run: total,
                applied,
                failed_at: None,
                error: None,
            },
            skipped,
        }
    }

    async fn install_step(
        &self,
        step: &Step,
        run: &mut InstallRun<'_>,
        applied: &mut Vec<String>,
    ) -> AppResult<()> {
        let ctx = RunContext::default();
        let step = expand_step(step, &run.env.values)?;

        match &step {
            Step::Download {
                artifact: Some(key),
                to,
                sha256,
                ..
            } => {
                let pin = run.env.artifacts.get(key).ok_or_else(|| {
                    AppError::invalid(format!("The artifact '{key}' has no binding."))
                })?;

                if !is_sha256_hex(&pin.sha256) {
                    return Err(AppError::jail(format!(
                        "The artifact '{key}' has no pinned sha256."
                    )));
                }

                if sha256
                    .as_deref()
                    .is_some_and(|s| !s.eq_ignore_ascii_case(&pin.sha256))
                {
                    return Err(AppError::jail(format!(
                        "The artifact '{key}' is bound to another sha256 than the step pins."
                    )));
                }

                let bound = Step::Download {
                    url: pin.url.clone(),
                    artifact: None,
                    to: to.clone(),
                    sha256: Some(pin.sha256.clone()),
                    max_bytes: pin.size,
                    mirrors: pin.mirrors.clone(),
                };

                self.run_step(&bound, &ctx, applied, Some(run)).await?;

                let target = self.jail.resolve(to, true)?;
                run.pinned
                    .insert(target.clone(), pin.sha256.to_ascii_lowercase());
                run.artifacts.insert(key.clone(), target);
            }

            Step::Download {
                to,
                sha256: Some(sha),
                ..
            } => {
                self.run_step(&step, &ctx, applied, Some(run)).await?;

                let target = self.jail.resolve(to, true)?;
                run.pinned.insert(target, sha.trim().to_ascii_lowercase());
            }

            other => self.run_step(other, &ctx, applied, Some(run)).await?,
        }

        Ok(())
    }
}

/// The step with the general placeholders filled in every string. A
/// `runInstaller` keeps its `program`, `args` and `cwd` raw: those take the
/// installer-only placeholders and are expanded by [`run_installer_step`].
fn expand_step(step: &Step, values: &Values) -> AppResult<Step> {
    let unresolved = |e: crate::layers::placeholders::Unresolved| AppError::invalid(e.to_string());

    if let Step::RunInstaller {
        program,
        args,
        cwd,
        timeout_secs,
        network,
        expect_files,
    } = step
    {
        return Ok(Step::RunInstaller {
            program: program.clone(),
            args: args.clone(),
            cwd: cwd.clone(),
            timeout_secs: *timeout_secs,
            network: *network,
            expect_files: expect_files
                .iter()
                .map(|p| {
                    Ok(PathRef {
                        root: p.root,
                        path: values.expand(&p.path).map_err(unresolved)?,
                    })
                })
                .collect::<AppResult<_>>()?,
        });
    }

    let raw = serde_json::to_value(step).map_err(|e| AppError::internal(e.to_string()))?;
    let filled = values.expand_json(&raw).map_err(unresolved)?;

    serde_json::from_value(filled)
        .map_err(|e| AppError::invalid(format!("Step is invalid after expansion: {e}")))
}

/// The four install-only steps. `step` is already expanded.
pub(crate) async fn run(
    exec: &Executor<'_>,
    step: &Step,
    run: &InstallRun<'_>,
    applied: &mut Vec<String>,
) -> AppResult<()> {
    match step {
        Step::RequireFile { path, sha256 } => {
            let target = exec.jail.resolve(path, false)?;

            if !target.is_file() {
                return Err(AppError::invalid(format!("{} is missing.", path.path)));
            }

            if let Some(want) = sha256 {
                let got = sha256_file(&target)?;

                if !got.eq_ignore_ascii_case(want.trim()) {
                    return Err(AppError::jail(format!(
                        "{} differs: expected {want}, got {got}.",
                        path.path
                    )));
                }
            }

            Ok(())
        }

        Step::MergeJson {
            base,
            overlay,
            to,
            rules,
        } => {
            let base_doc = read_json(&exec.jail.resolve(base, false)?)?;
            let overlay_doc = read_json(&exec.jail.resolve(overlay, false)?)?;
            let merged = merge_json(&base_doc, &overlay_doc, rules)?;
            let target = exec.jail.resolve(to, true)?;

            ensure_parent(&target)?;
            std::fs::write(&target, serde_json::to_vec_pretty(&merged)?)?;
            applied.push(display(&target));

            Ok(())
        }

        Step::DownloadList { .. } => download_list(exec, step, run, applied).await,

        Step::RunInstaller { .. } => run_installer_step(exec, step, run, applied).await,

        other => Err(AppError::internal(format!(
            "{} is not an install-only step.",
            other.action_name()
        ))),
    }
}

// ------------------------------------------------------------ requireFile

fn sha256_file(path: &Path) -> AppResult<String> {
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

    Ok(hex::encode(hasher.finalize()))
}

fn read_json(path: &Path) -> AppResult<Value> {
    let meta = std::fs::metadata(path)
        .map_err(|e| AppError::invalid(format!("{}: {e}", display(path))))?;

    if meta.len() > MAX_DOC_BYTES {
        return Err(AppError::jail(format!(
            "{} is too large to read.",
            display(path)
        )));
    }

    serde_json::from_slice(&std::fs::read(path)?)
        .map_err(|e| AppError::invalid(format!("{} is not valid JSON: {e}", display(path))))
}

// --------------------------------------------------------------- mergeJson

/// `overlay` deep-merged into `base` (objects merge key by key, anything else
/// is the overlay's value), then each rule applied at its pointer to the two
/// ORIGINAL values there. A Mojang `inheritsFrom` is `libraries` appended and
/// everything else, `mainClass` included, from the child.
pub fn merge_json(base: &Value, overlay: &Value, rules: &[MergeRule]) -> AppResult<Value> {
    let mut out = deep_merge(base, overlay);

    for rule in rules {
        if !rule.pointer.is_empty() && !rule.pointer.starts_with('/') {
            return Err(AppError::invalid(format!(
                "mergeJson: '{}' is not a JSON pointer.",
                rule.pointer
            )));
        }

        let b = base.pointer(&rule.pointer);
        let o = overlay.pointer(&rule.pointer);

        let value = match (rule.mode, b, o) {
            (_, None, None) => continue,
            (MergeMode::Replace, _, Some(o)) => o.clone(),
            (MergeMode::Replace, Some(b), None) => b.clone(),
            (mode, b, o) => {
                let as_list = |v: Option<&Value>| -> AppResult<Vec<Value>> {
                    match v {
                        None => Ok(Vec::new()),
                        Some(Value::Array(a)) => Ok(a.clone()),
                        Some(_) => Err(AppError::invalid(format!(
                            "mergeJson: '{}' is not an array on both sides.",
                            rule.pointer
                        ))),
                    }
                };
                let (b, o) = (as_list(b)?, as_list(o)?);

                Value::Array(match mode {
                    MergeMode::Append => b.into_iter().chain(o).collect(),
                    MergeMode::Prepend => o.into_iter().chain(b).collect(),
                    _ => {
                        let mut all = b;

                        for v in o {
                            if !all.contains(&v) {
                                all.push(v);
                            }
                        }

                        all
                    }
                })
            }
        };

        set_pointer(&mut out, &rule.pointer, value)?;
    }

    Ok(out)
}

fn deep_merge(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(b), Value::Object(o)) => {
            let mut out = b.clone();

            for (k, v) in o {
                let merged = match b.get(k) {
                    Some(bv) => deep_merge(bv, v),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }

            Value::Object(out)
        }
        (_, o) => o.clone(),
    }
}

/// Sets the value at an RFC 6901 pointer, creating missing objects on the
/// way. An array step must name an existing index.
fn set_pointer(doc: &mut Value, pointer: &str, value: Value) -> AppResult<()> {
    if pointer.is_empty() {
        *doc = value;
        return Ok(());
    }

    let parts: Vec<String> = pointer[1..]
        .split('/')
        .map(|p| p.replace("~1", "/").replace("~0", "~"))
        .collect();
    let (last, parents) = parts.split_last().expect("split always yields one part");
    let bad = || AppError::invalid(format!("mergeJson: cannot set '{pointer}'."));
    let mut cursor = doc;

    for part in parents {
        cursor = match cursor {
            Value::Object(map) => map
                .entry(part.clone())
                .or_insert_with(|| Value::Object(Default::default())),
            Value::Array(items) => part
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get_mut(i))
                .ok_or_else(bad)?,
            _ => return Err(bad()),
        };
    }

    match cursor {
        Value::Object(map) => {
            map.insert(last.clone(), value);
        }
        Value::Array(items) => {
            let slot = last
                .parse::<usize>()
                .ok()
                .and_then(|i| items.get_mut(i))
                .ok_or_else(bad)?;
            *slot = value;
        }
        _ => return Err(bad()),
    }

    Ok(())
}

// ------------------------------------------------------------ downloadList

/// One entry of a list, checked before anything is fetched.
struct ListEntry {
    url: String,
    target: PathBuf,
    sha1: Option<String>,
    sha256: Option<String>,
    size: Option<u64>,
}

/// A field of one entry: a JSON pointer, or a plain key.
fn field<'v>(entry: &'v Value, name: &str) -> Option<&'v Value> {
    if name.starts_with('/') {
        entry.pointer(name)
    } else {
        entry.get(name)
    }
}

async fn download_list(
    exec: &Executor<'_>,
    step: &Step,
    run: &InstallRun<'_>,
    applied: &mut Vec<String>,
) -> AppResult<()> {
    let Step::DownloadList {
        list,
        pointer,
        url_field,
        path_field,
        sha1_field,
        sha256_field,
        size_field,
        to,
        max,
    } = step
    else {
        return Err(AppError::internal("download_list on another step"));
    };

    if sha1_field.is_none() && sha256_field.is_none() {
        return Err(AppError::invalid(
            "downloadList needs sha1Field or sha256Field.",
        ));
    }

    /*
     * Provenance first. The list decides which URLs are fetched and which
     * digests they are held to, so it must be a file this run downloaded
     * against a pin, and still have that pin's bytes now.
     */
    let list_path = exec.jail.resolve(list, false)?;
    let pinned = run.pinned.get(&list_path).ok_or_else(|| {
        AppError::jail(format!(
            "downloadList: {} was not written by a pinned download in this install.",
            list.path
        ))
    })?;

    if sha256_file(&list_path)? != *pinned {
        return Err(AppError::jail(format!(
            "downloadList: {} changed after its pinned download.",
            list.path
        )));
    }

    let doc = read_json(&list_path)?;
    let items = doc
        .pointer(pointer)
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::invalid(format!("downloadList: '{pointer}' is not an array.")))?;

    let max = max.unwrap_or(DEFAULT_LIST_MAX);

    if max != 0 && items.len() as u64 > max {
        return Err(AppError::jail(format!(
            "downloadList: {} entries is more than the {max} allowed.",
            items.len()
        )));
    }

    let mut entries = Vec::with_capacity(items.len());

    for (i, item) in items.iter().enumerate() {
        let text = |name: &Option<String>| -> Option<String> {
            name.as_deref()
                .and_then(|n| field(item, n))
                .and_then(Value::as_str)
                .map(|s| s.trim().to_ascii_lowercase())
        };

        // An entry with nothing to fetch (a Mojang library that only has
        // natives classifiers, say) is not a download.
        let Some(url) = field(item, url_field).and_then(Value::as_str) else {
            continue;
        };

        let path = field(item, path_field)
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid(format!("downloadList: entry {i} has no path.")))?;

        let sha1 = text(sha1_field);
        let sha256 = text(sha256_field);

        if sha1.is_none() && sha256.is_none() {
            return Err(AppError::jail(format!(
                "downloadList: entry {i} has no hash."
            )));
        }

        if sha1.as_deref().is_some_and(|s| !is_hex(s, 40))
            || sha256.as_deref().is_some_and(|s| !is_hex(s, 64))
        {
            return Err(AppError::jail(format!(
                "downloadList: entry {i} has a malformed hash."
            )));
        }

        // Jail-checked under `to`: the entry's path is joined below it, so
        // `..` or an absolute path is refused by the same lexical check.
        let under = if to.path.is_empty() {
            path.to_string()
        } else {
            format!("{}/{path}", to.path.trim_end_matches('/'))
        };
        let target = exec.jail.resolve(
            &PathRef {
                root: to.root,
                path: under,
            },
            true,
        )?;

        let base = exec.jail.resolve(to, true)?;

        if !target.starts_with(&base) || target == base {
            return Err(AppError::jail(format!(
                "downloadList: entry {i} escapes its folder."
            )));
        }

        exec.check_url(url)?;

        entries.push(ListEntry {
            url: url.to_string(),
            target,
            sha1,
            sha256,
            size: size_field
                .as_deref()
                .and_then(|n| field(item, n))
                .and_then(Value::as_u64),
        });
    }

    let ctx = RunContext::default();

    for entry in entries {
        // A file already there with the right digest is not fetched again.
        if entry.target.is_file() && entry_matches(&entry)? {
            applied.push(display(&entry.target));
            continue;
        }

        exec.download(
            &entry.url,
            &entry.target,
            entry.sha256.as_deref(),
            entry.size,
            &ctx,
            0,
        )
        .await?;

        if !entry_matches(&entry)? {
            let _ = std::fs::remove_file(&entry.target);

            return Err(AppError::jail(format!(
                "{}: downloaded file did not match its sha1.",
                entry.url
            )));
        }

        applied.push(display(&entry.target));
    }

    Ok(())
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn entry_matches(entry: &ListEntry) -> AppResult<bool> {
    if let Some(size) = entry.size {
        if std::fs::metadata(&entry.target)?.len() != size {
            return Ok(false);
        }
    }

    if let Some(want) = &entry.sha256 {
        return Ok(sha256_file(&entry.target)? == *want);
    }

    match &entry.sha1 {
        Some(want) => Ok(sha1_hex(&std::fs::read(&entry.target)?) == *want),
        None => Ok(false),
    }
}

/// SHA-1 (FIPS 180-4), for list entries that publish only that. The list
/// itself is sha256-pinned, so this is integrity against a corrupt or
/// swapped transfer, not the root of trust. Kept here rather than adding a
/// crate for forty lines.
pub fn sha1_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);

    msg.push(0x80);

    while msg.len() % 64 != 56 {
        msg.push(0);
    }

    msg.extend_from_slice(&bit_len.to_be_bytes());

    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 80];

        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }

        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = h;

        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);

            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }

        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }

    h.iter().map(|x| format!("{x:08x}")).collect()
}

// ------------------------------------------------------------ runInstaller

/// One installer process, fully decided: what [`run_installer`] starts.
#[derive(Clone)]
pub struct InstallerSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// The WHOLE environment. Nothing of the app's own is inherited.
    pub env: Vec<(String, String)>,
    /// stdout and stderr, interleaved by line.
    pub log: PathBuf,
    /// `None` is no timeout.
    pub timeout: Option<Duration>,
    pub on_output: Option<OutputSink>,
}

/// The environment of an installer: `PATH` is the runtime's folder, home and
/// temp point into `work/`, `JAVA_HOME` for Java, and the proxy variables.
/// [`FORBIDDEN_ENV`] never appears, even if a proxy setting names it.
pub fn installer_env(
    program: &Path,
    java: bool,
    work: &Path,
    proxy: &[(String, String)],
) -> Vec<(String, String)> {
    let bin = program.parent().unwrap_or(Path::new(""));
    let home = work.join("home").to_string_lossy().into_owned();
    let tmp = work.join("tmp").to_string_lossy().into_owned();
    let mut env = vec![("PATH".to_string(), bin.to_string_lossy().into_owned())];

    for key in ["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA"] {
        env.push((key.to_string(), home.clone()));
    }

    for key in ["TMP", "TEMP", "TMPDIR"] {
        env.push((key.to_string(), tmp.clone()));
    }

    if java {
        if let Some(java_home) = bin.parent() {
            env.push(("JAVA_HOME".into(), java_home.to_string_lossy().into_owned()));
        }
    }

    /*
     * Windows only: without `SystemRoot` a process cannot load Winsock, so a
     * Java installer that downloads its libraries fails with an opaque
     * socket error. It names the OS folder and nothing the user controls.
     */
    #[cfg(windows)]
    if let Ok(root) = std::env::var("SystemRoot") {
        env.push(("SystemRoot".into(), root));
    }

    for (k, v) in proxy {
        if !FORBIDDEN_ENV.iter().any(|f| f.eq_ignore_ascii_case(k)) {
            env.push((k.clone(), v.clone()));
        }
    }

    env
}

/// Starts the installer and waits for it.
///
/// - The environment is exactly `spec.env`, stdin is null, and on Windows no
///   console window opens.
/// - It leads its own process group (Unix), so a timeout kills everything it
///   started; the group is killed again after a normal exit so a grandchild
///   it left behind does not outlive the step.
/// - Each line of stdout and stderr is appended to `spec.log` and handed to
///   `spec.on_output`.
///
/// Blocking: call it from `spawn_blocking`.
pub fn run_installer(spec: &InstallerSpec) -> AppResult<()> {
    ensure_parent(&spec.log)?;

    let log = Arc::new(Mutex::new(std::fs::File::create(&spec.log)?));
    let mut cmd = Command::new(&spec.program);

    cmd.args(&spec.args)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let mut child = cmd.spawn().map_err(|e| {
        AppError::invalid(format!("Could not start {}: {e}", display(&spec.program)))
    })?;
    let pid = child.id();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let mut readers = 0;

    let streams: [Option<Box<dyn Read + Send>>; 2] = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    ];

    for stream in streams.into_iter().flatten() {
        let log = log.clone();
        let sink = spec.on_output.clone();
        let done = done_tx.clone();

        readers += 1;
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line = Vec::new();

            while reader.read_until(b'\n', &mut line).unwrap_or(0) > 0 {
                if let Ok(mut file) = log.lock() {
                    let _ = file.write_all(&line);
                }

                if let Some(sink) = &sink {
                    sink(String::from_utf8_lossy(&line).trim_end());
                }

                line.clear();
            }

            let _ = done.send(());
        });
    }

    drop(done_tx);

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }

        if spec.timeout.is_some_and(|t| started.elapsed() >= t) {
            crate::console::kill_tree(pid);
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }

        std::thread::sleep(Duration::from_millis(50));
    };

    // Whatever it left running dies with the step.
    crate::console::kill_tree(pid);

    // The pipes close when the last holder dies; a holder that escaped the
    // group must not hang the install, so the readers get a bounded wait.
    for _ in 0..readers {
        if done_rx.recv_timeout(Duration::from_secs(5)).is_err() {
            break;
        }
    }

    match status {
        None => Err(AppError::invalid(format!(
            "The installer did not finish within {}s and was stopped. See {}.",
            spec.timeout.unwrap_or_default().as_secs(),
            display(&spec.log)
        ))),
        Some(status) if !status.success() => Err(AppError::invalid(format!(
            "The installer failed ({status}). See {}.",
            display(&spec.log)
        ))),
        Some(_) => Ok(()),
    }
}

async fn run_installer_step(
    exec: &Executor<'_>,
    step: &Step,
    run: &InstallRun<'_>,
    applied: &mut Vec<String>,
) -> AppResult<()> {
    let Step::RunInstaller {
        program,
        args,
        cwd,
        timeout_secs,
        expect_files,
        ..
    } = step
    else {
        return Err(AppError::internal("run_installer_step on another step"));
    };

    if cwd != "{layerWork}" {
        return Err(AppError::jail("runInstaller.cwd must be {layerWork}."));
    }

    let layer = exec
        .jail
        .root_path(FsRoot::Layer)
        .ok_or_else(|| AppError::jail("runInstaller needs the layer root."))?;
    let files = layer.join("files");
    let work = layer.join("work");

    for dir in [&files, &work, &work.join("home"), &work.join("tmp")] {
        std::fs::create_dir_all(dir)?;
    }

    // `{runtime.<kind>}` or `{artifact.<key>}`, and nothing else: no PATH
    // lookup, no absolute path.
    let name = program
        .strip_prefix('{')
        .and_then(|p| p.strip_suffix('}'))
        .and_then(|p| p.split_once('.'))
        .filter(|(_, k)| crate::layers::placeholders::is_ident(k));

    let (exe, java) = match name {
        Some(("runtime", kind)) => {
            let rt = run.env.runtimes.get(kind).ok_or_else(|| {
                AppError::invalid(format!("The runtime '{kind}' is not installed."))
            })?;
            (rt.exe.clone(), rt.java)
        }
        Some(("artifact", key)) => (
            run.artifacts.get(key).cloned().ok_or_else(|| {
                AppError::invalid(format!("The artifact '{key}' was not downloaded."))
            })?,
            false,
        ),
        _ => {
            return Err(AppError::jail(
                "runInstaller.program must be {runtime.<kind>} or {artifact.<key>}.",
            ))
        }
    };

    let mut values = run.env.values.clone();

    for (key, path) in &run.artifacts {
        values.set(format!("artifact.{key}"), path.to_string_lossy());
    }

    values.set("layerFiles", files.to_string_lossy());
    values.set("layerWork", work.to_string_lossy());

    let mut argv = Vec::with_capacity(args.len() + 2);

    if java {
        argv.push(format!("-Djava.io.tmpdir={}", work.join("tmp").display()));
        argv.push(format!("-Duser.home={}", work.join("home").display()));
    }

    for arg in args {
        argv.push(
            values
                .expand(arg)
                .map_err(|e| AppError::invalid(e.to_string()))?,
        );
    }

    let secs = timeout_secs.unwrap_or(DEFAULT_INSTALLER_TIMEOUT_SECS);
    let spec = InstallerSpec {
        env: installer_env(&exe, java, &work, &run.env.proxy),
        program: exe,
        args: argv,
        cwd: work.clone(),
        log: work.join("installer.log"),
        timeout: (secs > 0).then(|| Duration::from_secs(secs)),
        on_output: run.env.on_output.clone(),
    };

    audit!(
        exec.audit,
        Security,
        Install,
        "install.run_installer",
        format!("{} {}", display(&spec.program), spec.args.join(" ")),
        plugin = exec.manifest.id
    );

    tokio::task::spawn_blocking(move || run_installer(&spec))
        .await
        .map_err(|e| AppError::internal(format!("The installer task failed: {e}")))??;

    for expected in expect_files {
        if expected.root != FsRoot::Layer {
            return Err(AppError::jail(
                "runInstaller.expectFiles must be under layer.",
            ));
        }

        let path = exec.jail.resolve(expected, false)?;

        if !path.exists() {
            return Err(AppError::invalid(format!(
                "The installer finished but did not produce {}.",
                expected.path
            )));
        }
    }

    applied.push(display(&files));

    Ok(())
}

#[cfg(test)]
mod tests;
