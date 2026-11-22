//! Resolve a sandbox's game version and loader to an [`InstallPlan`]
//! (docs/loaders-and-versions.md, "Deploy consumption", phase 1).
//!
//! The plan source is the catalog of an app (`versions`, `loaderVersions`,
//! `versionBindings`, `builds`) plus the install manifests the builds name.
//! In this unit the only source is the one compiled into the app
//! ([`PlanSource::compiled`], from `manifests/**`); the signed catalog served
//! by the website is a later unit. Tests build their own source with
//! [`PlanSource::from_entries`].
//!
//! A plan has one layer per manifest: the game version's layer first, then the
//! loader's. Each layer's hash covers its manifest, the values its steps see,
//! the pinned artifact hashes, the platform and the side, and nothing about
//! the sandbox, so two sandboxes on the same build share one layer folder.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::deploy::json_patch::JsonPatchSpec;
use crate::error::{AppError, AppResult};
use crate::layers::canonical;
use crate::layers::lint::{check_manifest, ExecKey, LintContext};
use crate::layers::placeholders::Values;
use crate::plugins::manifest::{parse_install_steps, InstallStep, When, WhenContext};
use crate::plugins::steps_install::ArtifactPin;

include!(concat!(env!("OUT_DIR"), "/builtin_install_manifests.rs"));

/// Where a plan's manifests came from. Exec steps (`runInstaller`) are only
/// accepted from a source that is not `Local`, and only when signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Trust {
    /// Compiled into this build of the app.
    Compiled,
    /// Fetched from the TMC website and verified.
    Official,
    /// Read from the user's disk (a manifest under development).
    Local,
}

/// One install manifest of a [`PlanSource`].
#[derive(Debug, Clone)]
pub struct ManifestEntry {
    pub doc: Value,
    pub text: String,
    pub sig: Option<Value>,
}

/// A runtime a plan can download: `runtimes/<kind>.snapshot.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeOffer {
    pub kind: String,
    pub major: String,
    pub platform: String,
    #[serde(default)]
    pub vendor_label: Option<String>,
    /// The executable, relative to the extracted root (`bin/java`).
    pub entry: String,
    #[serde(default)]
    pub strip_single_root: bool,
    pub artifact: ArtifactBinding,
}

/// A pinned download, as the catalog's bindings carry it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactBinding {
    pub url: String,
    #[serde(default)]
    pub mirrors: Vec<String>,
    pub sha256: String,
    #[serde(default)]
    pub size: Option<u64>,
}

impl ArtifactBinding {
    pub fn pin(&self) -> ArtifactPin {
        ArtifactPin {
            url: self.url.clone(),
            sha256: self.sha256.clone(),
            mirrors: self.mirrors.clone(),
            size: self.size,
        }
    }
}

/// The catalogs, manifests, exec keys and runtime offers a plan is resolved
/// against.
#[derive(Debug, Clone)]
pub struct PlanSource {
    pub trust: Trust,
    /// By manifest id (`minecraft/loader/forge`).
    pub manifests: BTreeMap<String, ManifestEntry>,
    pub keys: Vec<ExecKey>,
    /// By app slug.
    pub catalogs: BTreeMap<String, Value>,
    pub runtimes: Vec<RuntimeOffer>,
}

impl PlanSource {
    /// Read a source from `(path, text)` pairs laid out like `manifests/`:
    /// `keys/exec-keys.json`, `<app>/catalog.snapshot.json`,
    /// `**/*.manifest.json` with an optional `.sig` beside it, and
    /// `runtimes/<kind>.snapshot.json`.
    pub fn from_entries<'a>(
        trust: Trust,
        entries: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> AppResult<Self> {
        let mut source = PlanSource {
            trust,
            manifests: BTreeMap::new(),
            keys: Vec::new(),
            catalogs: BTreeMap::new(),
            runtimes: Vec::new(),
        };
        let mut sigs: HashMap<String, Value> = HashMap::new();
        let mut by_path: HashMap<String, String> = HashMap::new();

        let bad = |rel: &str, e: &dyn std::fmt::Display| {
            AppError::invalid(format!("The install manifest file {rel} is invalid: {e}"))
        };

        for (rel, text) in entries {
            if rel == "keys/exec-keys.json" {
                source.keys = serde_json::from_str(text).map_err(|e| bad(rel, &e))?;
            } else if let Some(sig_of) = rel.strip_suffix(".sig") {
                let sig = canonical::parse(text).unwrap_or(Value::Null);
                sigs.insert(sig_of.to_string(), sig);
            } else if rel.ends_with(".manifest.json") {
                let doc = canonical::parse(text).map_err(|e| bad(rel, &e))?;
                let id = doc
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| bad(rel, &"it has no id"))?
                    .to_string();
                by_path.insert(rel.to_string(), id.clone());
                source.manifests.insert(
                    id,
                    ManifestEntry {
                        doc,
                        text: text.to_string(),
                        sig: None,
                    },
                );
            } else if let Some(rest) = rel.strip_prefix("runtimes/") {
                if rest.ends_with(".snapshot.json") {
                    let doc: Value = serde_json::from_str(text).map_err(|e| bad(rel, &e))?;
                    let list = doc.get("runtimes").cloned().unwrap_or(doc);
                    let offers: Vec<RuntimeOffer> =
                        serde_json::from_value(list).map_err(|e| bad(rel, &e))?;
                    source.runtimes.extend(offers);
                }
            } else if let Some(app) = rel.strip_suffix("/catalog.snapshot.json") {
                let doc: Value = serde_json::from_str(text).map_err(|e| bad(rel, &e))?;
                source.catalogs.insert(app.to_string(), doc);
            }
        }

        for (path, sig) in sigs {
            if let Some(entry) = by_path
                .get(&path)
                .and_then(|id| source.manifests.get_mut(id))
            {
                entry.sig = Some(sig);
            }
        }

        Ok(source)
    }

    /// The source compiled into this build (`manifests/**`).
    pub fn compiled() -> &'static PlanSource {
        static SOURCE: OnceLock<PlanSource> = OnceLock::new();
        SOURCE.get_or_init(|| {
            PlanSource::from_entries(Trust::Compiled, BUILTIN_INSTALL_MANIFESTS.iter().copied())
                .expect("the compiled install manifests are checked by manifests:check")
        })
    }

    fn known_ids(&self, app: &str) -> HashSet<String> {
        let prefix = format!("{app}/");
        self.manifests
            .keys()
            .filter(|id| id.starts_with(&prefix))
            .cloned()
            .collect()
    }
}

/// What a plan is resolved for.
#[derive(Debug, Clone, Default)]
pub struct PlanRequest {
    pub app_slug: String,
    pub game_version: String,
    /// `None`, `""` or `vanilla` for no loader.
    pub loader: Option<String>,
    pub loader_version: Option<String>,
    /// `client` or `server`.
    pub side: String,
    /// [`current_platform`] unless a test says otherwise.
    pub platform: String,
    pub sandbox_id: String,
    pub sandbox_name: String,
    /// The game folder the deploy writes to, for the `{targetDir}` of patches.
    pub target_dir: String,
}

/// One layer of a plan.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedLayer {
    pub manifest_key: String,
    pub revision: u32,
    pub title: String,
    pub layer_hash: String,
    /// The layer runs an installer program.
    pub exec: bool,
    /// The steps that run for this side and platform, for the preview.
    pub steps: Vec<Value>,
    #[serde(skip)]
    pub install_steps: Vec<InstallStep>,
    /// For each of `install_steps`, its index in `steps`, or `None` for a step
    /// that does not apply here: how a progress event names a step.
    #[serde(skip)]
    pub preview_index: Vec<Option<usize>>,
    pub values: BTreeMap<String, String>,
    #[serde(skip)]
    pub artifacts: HashMap<String, ArtifactPin>,
    pub download_hosts: Vec<String>,
}

impl PlannedLayer {
    pub fn values(&self) -> Values {
        Values(self.values.clone().into_iter().collect())
    }
}

/// A runtime a plan needs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedRuntime {
    pub kind: String,
    pub major: String,
    /// `install` (an installer runs on it) or `launch`.
    #[serde(rename = "for")]
    pub for_: String,
    pub offer: Option<RuntimeOffer>,
}

/// A consent a plan asks for, as the dialog shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedConsent {
    /// `exec:<manifest>`, `runtime:<kind>-<major>`, or a manifest consent id.
    pub key: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Everything a deploy installs below the user's mods.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPlan {
    pub plan_hash: String,
    pub app_slug: String,
    pub trust: Trust,
    pub side: String,
    pub platform: String,
    pub layers: Vec<PlannedLayer>,
    pub runtimes: Vec<PlannedRuntime>,
    pub consents: Vec<PlannedConsent>,
    /// Some layer runs an installer program.
    pub exec: bool,
    pub launch_vars: BTreeMap<String, String>,
    pub patches: Vec<JsonPatchSpec>,
}

impl InstallPlan {
    /// The ids of the manifest consents (`eula`), which the steps' `when`
    /// matches once the user agreed.
    pub fn consent_ids(&self) -> BTreeSet<String> {
        self.consents.iter().map(|c| c.key.clone()).collect()
    }

    /// The runtimes the build downloads.
    pub fn install_runtimes(&self) -> impl Iterator<Item = &PlannedRuntime> {
        self.runtimes.iter().filter(|r| r.for_ == "install")
    }
}

/// `linux-x64`, `windows-x64`, `macos-arm64`: the `platform` of manifests and
/// runtime offers.
pub fn current_platform() -> String {
    let os = std::env::consts::OS;
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{os}-{arch}")
}

/// True when `loader` means "no loader".
pub fn is_vanilla(loader: Option<&str>) -> bool {
    loader.is_none_or(|l| l.is_empty() || l.eq_ignore_ascii_case("vanilla"))
}

/// Resolve `req` against `source`.
///
/// `Ok(None)` means the app has no managed install for this version (no
/// catalog, an unknown or unmanaged version): the deploy goes on with mods
/// only, as before layers existed. A withdrawn version or build, a missing
/// build, a manifest that fails its checks and an unsigned installer are
/// errors.
pub fn resolve(source: &PlanSource, req: &PlanRequest) -> AppResult<Option<InstallPlan>> {
    let Some(catalog) = source.catalogs.get(&req.app_slug) else {
        return Ok(None);
    };
    let version_key = format!("{}/version", req.app_slug);
    let Some(version_entry) = source.manifests.get(&version_key) else {
        return Ok(None);
    };
    let app_title = version_entry
        .doc
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(&req.app_slug)
        .to_string();

    let items = catalog
        .pointer("/versions/items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let Some(version) = items
        .iter()
        .find(|v| v.get("versionId").and_then(Value::as_str) == Some(req.game_version.as_str()))
    else {
        return Ok(None);
    };
    if let Some(note) = withdrawn(version) {
        return Err(AppError::invalid(format!(
            "{app_title} {} was withdrawn{note}. Pick another version.",
            req.game_version
        )));
    }
    if version.get("managed").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }

    let version_bindings = catalog
        .pointer(&format!(
            "/versionBindings/{}",
            escape_pointer(&req.game_version)
        ))
        .cloned()
        .unwrap_or(json!({}));

    // The layers to build: (manifest key, its bindings, the build's revision).
    let mut picks: Vec<(String, Value, Option<u64>)> =
        vec![(version_key.clone(), version_bindings.clone(), None)];

    let loader = req
        .loader
        .as_deref()
        .filter(|_| !is_vanilla(req.loader.as_deref()));
    if let Some(loader) = loader {
        if let Some(pick) = pick_build(catalog, req, loader)? {
            picks.push(pick);
        }
    }

    let known_ids = source.known_ids(&req.app_slug);
    let mut layers = Vec::new();
    let mut runtimes: Vec<PlannedRuntime> = Vec::new();
    let mut consents: Vec<PlannedConsent> = Vec::new();
    let mut launch_vars = BTreeMap::new();
    let mut patches = Vec::new();
    let mut values = Values::default();
    values.set("gameVersion", req.game_version.clone());
    values.set("platform", req.platform.clone());
    values.set("side", req.side.clone());
    absorb_vars(&mut values, &version_bindings);

    let when_all = WhenContext {
        side: req.side.clone(),
        platform: req.platform.clone(),
        consents: BTreeSet::new(),
    };

    for (index, (key, bindings, build_revision)) in picks.iter().enumerate() {
        let entry = source.manifests.get(key).ok_or_else(|| {
            AppError::invalid(format!(
                "The catalog names the install manifest {key}, which this app does not have. Update the app."
            ))
        })?;
        let doc = &entry.doc;
        let title = doc
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(key)
            .to_string();
        let revision = doc.get("revision").and_then(Value::as_u64).unwrap_or(0);

        if let Some(want) = build_revision {
            if *want != revision {
                return Err(AppError::invalid(format!(
                    "The catalog wants revision {want} of {key}, and this app has revision {revision}. Update the app."
                )));
            }
        }

        let problems = check_manifest(
            &entry.text,
            LintContext {
                known_ids: &known_ids,
                sig: entry.sig.as_ref(),
                keys: &source.keys,
            },
        );
        if !problems.is_empty() {
            let list: Vec<String> = problems
                .iter()
                .map(|p| format!("{} at {}: {}", p.code, p.at, p.message))
                .collect();
            return Err(AppError::invalid(format!(
                "The install manifest {key} failed its checks: {}",
                list.join("; ")
            )));
        }

        let earlier: HashSet<&str> = picks[..index].iter().map(|(k, _, _)| k.as_str()).collect();
        for need in doc
            .get("requires")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let need = need.as_str().unwrap_or_default();
            if !earlier.contains(need) {
                return Err(AppError::invalid(format!(
                    "{key} needs {need} below it, and the plan does not have it."
                )));
            }
        }

        // Values: the version's, then this build's, then the manifest's vars.
        if index > 0 {
            if let Some(loader) = loader {
                values.set("loader", loader.to_string());
            }
            if let Some(v) = &req.loader_version {
                values.set("loaderVersion", v.clone());
            }
            absorb_vars(&mut values, bindings);
        }
        if let Some(vars) = doc.get("vars").and_then(Value::as_object) {
            for (name, template) in vars {
                let template = template.as_str().unwrap_or_default();
                let value = values.expand(template).map_err(|e| {
                    AppError::invalid(format!("{key}: the var {name} cannot be filled: {e}"))
                })?;
                set_var(&mut values, name, &value);
            }
        }

        // The steps that run here, and whether any runs a program.
        let raw_steps: Vec<Value> = doc
            .get("steps")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let install_steps = parse_install_steps(&Value::Array(raw_steps.clone()))?;
        let mut steps = Vec::new();
        let mut preview_index = Vec::new();
        let mut exec = false;
        let mut used_artifacts = BTreeSet::new();
        for (raw, step) in raw_steps.iter().zip(&install_steps) {
            if !applies(step.when.as_ref(), &when_all) {
                preview_index.push(None);
                continue;
            }
            preview_index.push(Some(steps.len()));
            if raw.get("action").and_then(Value::as_str) == Some("runInstaller") {
                exec = true;
            }
            if let Some(a) = raw.get("artifact").and_then(Value::as_str) {
                used_artifacts.insert(a.to_string());
            }
            steps.push(raw.clone());
        }
        if exec && source.trust == Trust::Local {
            return Err(AppError::invalid(format!(
                "{key} runs an installer program, and a local manifest may not."
            )));
        }

        // The pinned downloads, and the hosts they come from.
        let mut artifacts = HashMap::new();
        let mut hosts: BTreeSet<String> = doc
            .get("downloadHosts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        let declared = doc.get("artifacts").and_then(Value::as_object);
        for name in &used_artifacts {
            let decl = declared.and_then(|d| d.get(name));
            if decl.is_none() {
                return Err(AppError::invalid(format!(
                    "{key} downloads {name}, which it does not declare."
                )));
            }
            let binding = bindings
                .pointer(&format!("/artifacts/{}", escape_pointer(name)))
                .cloned()
                .ok_or_else(|| {
                    AppError::invalid(format!(
                        "The catalog has no pinned download of {name} for {title} on {app_title} {}.",
                        req.game_version
                    ))
                })?;
            let binding: ArtifactBinding = serde_json::from_value(binding).map_err(|e| {
                AppError::invalid(format!(
                    "The catalog's download of {name} for {title} is invalid: {e}"
                ))
            })?;
            for url in std::iter::once(&binding.url).chain(&binding.mirrors) {
                if let Some(host) = url::Url::parse(url)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_string))
                {
                    hosts.insert(host);
                }
            }
            artifacts.insert(name.clone(), binding.pin());
        }

        // Runtimes this layer needs.
        let reqs = doc
            .pointer("/requirements/runtimes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for r in reqs {
            let for_ = r
                .get("for")
                .and_then(Value::as_str)
                .unwrap_or("launch")
                .to_string();
            let kind = r
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let major = r.get("major").and_then(Value::as_str).unwrap_or_default();
            let install = for_ == "install";
            if install && !exec {
                continue;
            }
            let major = match values.expand(major) {
                Ok(m) => m,
                // A launch runtime is not managed yet; one the catalog does
                // not pin is skipped. An installer cannot run without its own.
                Err(_) if !install => continue,
                Err(e) => {
                    return Err(AppError::invalid(format!(
                        "{key} needs a {kind} runtime, and the catalog does not say which: {e}"
                    )))
                }
            };
            let offer = source
                .runtimes
                .iter()
                .find(|o| o.kind == kind && o.major == major && o.platform == req.platform)
                .cloned();
            if install && offer.is_none() {
                return Err(AppError::invalid(format!(
                    "{title} needs {kind} {major} to install, and no {kind} {major} is published for {} yet.",
                    req.platform
                )));
            }
            if let Some(existing) = runtimes
                .iter_mut()
                .find(|e| e.kind == kind && e.major == major)
            {
                if install {
                    existing.for_ = for_;
                    existing.offer = offer;
                }
                continue;
            }
            runtimes.push(PlannedRuntime {
                kind,
                major,
                for_,
                offer,
            });
        }

        // The manifest's own consents, for this side.
        for c in doc
            .get("consents")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let sides_ok = c
                .get("sides")
                .and_then(Value::as_array)
                .is_none_or(|s| s.iter().any(|x| x.as_str() == Some(req.side.as_str())));
            let Some(id) = c.get("id").and_then(Value::as_str) else {
                continue;
            };
            if sides_ok && !consents.iter().any(|k: &PlannedConsent| k.key == id) {
                consents.push(PlannedConsent {
                    key: id.to_string(),
                    text: c
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or(id)
                        .to_string(),
                    url: c.get("url").and_then(Value::as_str).map(str::to_string),
                });
            }
        }
        if exec {
            consents.push(PlannedConsent {
                key: format!("exec:{key}"),
                text: format!(
                    "{title} is installed by running its official installer program on this computer."
                ),
                url: None,
            });
        }

        // Launch vars: a later layer overrides an earlier one.
        for name in doc
            .pointer("/launch/vars")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(name) = name.as_str() else { continue };
            let value = values
                .0
                .get(&format!("var.{name}"))
                .or_else(|| values.0.get(name))
                .cloned();
            if let Some(value) = value {
                launch_vars.insert(name.to_string(), value);
            }
        }

        // The layer hash: nothing about the sandbox goes in.
        let layer_values: BTreeMap<String, String> = values
            .0
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let pinned: BTreeMap<&String, &String> = artifacts
            .iter()
            .map(|(k, p): (&String, &ArtifactPin)| (k, &p.sha256))
            .collect();
        let layer_hash = hash_value(&json!({
            "manifest": doc,
            "values": layer_values,
            "artifacts": pinned,
            "platform": req.platform,
            "side": req.side,
        }))?;

        layers.push(PlannedLayer {
            manifest_key: key.clone(),
            revision: revision as u32,
            title,
            layer_hash,
            exec,
            steps,
            install_steps,
            preview_index,
            values: layer_values,
            artifacts,
            download_hosts: hosts.into_iter().collect(),
        });
    }

    for rt in runtimes.iter().filter(|r| r.for_ == "install") {
        consents.push(PlannedConsent {
            key: format!("runtime:{}-{}", rt.kind, rt.major),
            text: format!(
                "{} {} ({}) is downloaded to run the installer.",
                rt.kind,
                rt.major,
                rt.offer
                    .as_ref()
                    .and_then(|o| o.vendor_label.clone())
                    .unwrap_or_default()
            ),
            url: None,
        });
    }

    // Patches, expanded with the sandbox's values.
    for (layer, (key, _, _)) in layers.iter().zip(&picks) {
        let doc = &source.manifests[key].doc;
        let mut v = layer.values();
        for (name, value) in &launch_vars {
            set_var(&mut v, name, value);
        }
        v.set("sandboxId", req.sandbox_id.clone());
        v.set(
            "sandboxName",
            req.sandbox_name.chars().take(64).collect::<String>(),
        );
        v.set("targetDir", req.target_dir.clone());
        for p in doc
            .get("deployPatches")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let when: Option<When> = p
                .get("when")
                .map(|w| serde_json::from_value(w.clone()))
                .transpose()
                .map_err(|e| {
                    AppError::invalid(format!("{key}: a deploy patch's when is invalid: {e}"))
                })?;
            if !applies(when.as_ref(), &when_all) {
                continue;
            }
            let fill = |field: &str| -> AppResult<Option<Value>> {
                p.get(field)
                    .map(|x| v.expand_json(x))
                    .transpose()
                    .map_err(|e| {
                        AppError::invalid(format!("{key}: a deploy patch cannot be filled: {e}"))
                    })
            };
            let text = |field: &str| -> AppResult<String> {
                Ok(fill(field)?
                    .and_then(|x| x.as_str().map(str::to_string))
                    .unwrap_or_default())
            };
            patches.push(JsonPatchSpec {
                mod_key: format!("patch:{key}"),
                path: text("path")?,
                ensure: fill("ensure")?,
                pointer: text("pointer")?,
                value: fill("value")?.unwrap_or(Value::Null),
            });
        }
    }

    let exec = layers.iter().any(|l| l.exec);
    let plan_hash = hash_value(&json!({
        "trust": source.trust,
        "layers": layers.iter().map(|l| json!([l.manifest_key, l.revision, l.layer_hash, l.exec])).collect::<Vec<_>>(),
        "runtimes": runtimes.iter().map(|r| json!([r.kind, r.major, r.for_, r.offer.as_ref().map(|o| &o.artifact.sha256)])).collect::<Vec<_>>(),
        "consents": consents.iter().map(|c| &c.key).collect::<Vec<_>>(),
        "exec": exec,
    }))?;

    Ok(Some(InstallPlan {
        plan_hash,
        app_slug: req.app_slug.clone(),
        trust: source.trust,
        side: req.side.clone(),
        platform: req.platform.clone(),
        layers,
        runtimes,
        consents,
        exec,
        launch_vars,
        patches,
    }))
}

/// The build for `loader` at the requested versions, or `None` when the
/// loader is not managed by the catalog or the version is a custom one.
fn pick_build(
    catalog: &Value,
    req: &PlanRequest,
    loader: &str,
) -> AppResult<Option<(String, Value, Option<u64>)>> {
    let Some(entry) = catalog
        .get("loaders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|l| {
            l.get("key")
                .and_then(Value::as_str)
                .is_some_and(|k| k.eq_ignore_ascii_case(loader))
        })
    else {
        return Ok(None);
    };
    if entry.get("managed").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    let name = entry.get("name").and_then(Value::as_str).unwrap_or(loader);
    let key = entry.get("key").and_then(Value::as_str).unwrap_or(loader);

    let Some(wanted) = req.loader_version.as_deref().filter(|v| !v.is_empty()) else {
        return Err(AppError::invalid(format!(
            "Pick a {name} version for this sandbox."
        )));
    };

    let listed = catalog
        .pointer(&format!("/loaderVersions/{}", escape_pointer(key)))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|v| v.get("version").and_then(Value::as_str) == Some(wanted));
    match listed {
        Some(v) => {
            if let Some(note) = withdrawn(v) {
                return Err(AppError::invalid(format!(
                    "This {name} version was withdrawn{note}. Pick another version."
                )));
            }
            if v.get("custom").and_then(Value::as_bool) == Some(true) {
                return Ok(None);
            }
        }
        None if entry.get("customVersions").and_then(Value::as_bool) == Some(true) => {
            return Ok(None)
        }
        None => {
            return Err(AppError::invalid(format!(
                "{name} has no version {wanted}. Pick another version."
            )))
        }
    }

    let build = catalog
        .get("builds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|b| {
            b.get("loader").and_then(Value::as_str) == Some(key)
                && b.get("loaderVersion").and_then(Value::as_str) == Some(wanted)
                && b.get("gameVersion").and_then(Value::as_str) == Some(req.game_version.as_str())
        })
        .ok_or_else(|| {
            AppError::invalid(format!(
                "No install of {name} {wanted} for {} is published yet. Pick another version.",
                req.game_version
            ))
        })?;

    Ok(Some((
        build
            .get("manifestKey")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        build.get("bindings").cloned().unwrap_or(json!({})),
        build.get("revision").and_then(Value::as_u64),
    )))
}

/// `Some(": <note>")` (or `Some("")`) when `item.deprecated.withdrawn`.
fn withdrawn(item: &Value) -> Option<String> {
    let d = item.get("deprecated").filter(|d| d.is_object())?;
    if d.get("withdrawn").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    Some(match d.get("note").and_then(Value::as_str) {
        Some(note) if !note.is_empty() => format!(": {note}"),
        _ => String::new(),
    })
}

fn absorb_vars(values: &mut Values, bindings: &Value) {
    for (name, value) in bindings
        .get("vars")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        if let Some(value) = value.as_str() {
            set_var(values, name, value);
        }
    }
}

/// A var is `{var.name}`; `launchVersionId` is also a general placeholder.
fn set_var(values: &mut Values, name: &str, value: &str) {
    values.set(format!("var.{name}"), value.to_string());
    if name == "launchVersionId" {
        values.set(name.to_string(), value.to_string());
    }
}

fn applies(when: Option<&When>, ctx: &WhenContext) -> bool {
    // The preview assumes every consent is given: a step behind one runs once
    // the user agreed, and the consent dialog lists it.
    when.is_none_or(|w| {
        let mut w = w.clone();
        w.consent = None;
        w.matches(ctx)
    })
}

fn escape_pointer(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

fn hash_value(v: &Value) -> AppResult<String> {
    let bytes = canonical::canonical_bytes(v)
        .map_err(|e| AppError::internal(format!("Could not hash the install plan: {}", e.0)))?;
    Ok(crate::layers::checks::hex(&Sha256::digest(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compiled_source_reads_every_manifest() {
        let s = PlanSource::compiled();
        assert!(s.manifests.contains_key("minecraft/version"));
        assert!(s.manifests.contains_key("minecraft/loader/forge"));
        assert!(s.manifests["minecraft/loader/forge"].sig.is_some());
        assert!(!s.keys.is_empty());
        assert!(s.catalogs.contains_key("minecraft"));
    }

    #[test]
    fn an_empty_catalog_plans_nothing() {
        let req = PlanRequest {
            app_slug: "minecraft".into(),
            game_version: "1.21.4".into(),
            side: "client".into(),
            platform: current_platform(),
            ..Default::default()
        };
        assert!(resolve(PlanSource::compiled(), &req).unwrap().is_none());
    }

    #[test]
    fn vanilla_means_no_loader() {
        assert!(is_vanilla(None));
        assert!(is_vanilla(Some("")));
        assert!(is_vanilla(Some("Vanilla")));
        assert!(!is_vanilla(Some("forge")));
    }
}
