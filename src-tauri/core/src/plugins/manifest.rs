//! What a plugin is allowed to say about itself.
//!
//! The central design decision, and the one everything else follows from:
//! **a plugin is data, never code.** There is no script engine, no WASM host,
//! no `exec` step. An installer describes a sequence of file operations; a
//! query plugin describes a datagram and how to read the reply; a theme
//! describes colour tokens. Everything a plugin can express is something this
//! crate implements and audits.
//!
//! That is a real limitation — a plugin cannot do anything we did not think of
//! — and it is the point. The alternative is running third-party code with the
//! user's full filesystem access on the same machine as their games, their save
//! files and their credentials, and no amount of isolation bolted on
//! afterwards recovers from that starting position.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

/// The only manifest version this build accepts.
pub const MANIFEST_VERSION: u32 = 1;

/// File a plugin bundle must contain at its root.
pub const MANIFEST_FILE: &str = "plugin.json";

/// Most mirrors one download step may list.
pub const MAX_MIRRORS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub manifest_version: u32,

    /// Reverse-DNS id. Also the on-disk directory name, which is why
    /// [`is_valid_plugin_id`] is strict about it.
    pub id: String,

    pub name: String,
    pub version: String,
    pub author: String,

    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,

    /// GameSpice app (game) ids this plugin handles. Empty means "any", which is
    /// only sensible for a theme.
    #[serde(default)]
    pub apps: Vec<i64>,

    pub permissions: Permissions,

    #[serde(default)]
    pub installer: Option<Installer>,
    #[serde(default)]
    pub server_query: Option<ServerQuery>,
    #[serde(default)]
    pub theme: Option<Theme>,

    /// A mod manager this app can read an existing library out of.
    ///
    /// The fourth plugin type, and the one that reads rather than writes:
    /// [`crate::plugins::managers`] describes where another manager keeps its
    /// mods, and the app copies from there into its own store. It declares no
    /// steps and needs no `fs` grant, because a scan never writes and the
    /// copy is performed by the app against paths the SCAN found — see that
    /// module's header for why a descriptor cannot name a path of its own.
    #[serde(default)]
    pub manager: Option<crate::plugins::managers::ManagerSpec>,
}

/// Everything a plugin may reach. Absent means denied — there is no wildcard
/// and no "inherit" that could widen a grant.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Permissions {
    /// Writable roots. Each entry names a base the app resolves (never a path
    /// the plugin supplies) plus an optional subdirectory under it.
    #[serde(default)]
    pub fs: Vec<FsGrant>,

    /// Hosts the plugin may download from. Exact hostnames or a single leading
    /// `*.` label — never a bare `*`, and never a scheme other than https.
    #[serde(default)]
    pub net: Vec<String>,

    /// UDP/TCP query targets are NOT listed here: a query plugin probes the
    /// server the user picked, which is not knowable at authoring time. The
    /// control there is [`crate::net::resolve_public`], which refuses anything
    /// that is not a public address.
    #[serde(default)]
    pub query: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FsRoot {
    /// The configured install directory for the app being handled. Resolved
    /// from `AppSettings::game_dirs`, so the USER chose it.
    GameDir,
    /// This plugin's own private directory under the app's data dir.
    PluginData,
    /// The user's configured download directory. Read-write, but shared with
    /// other plugins — anything durable belongs in `pluginData`.
    Downloads,
    /// The game's Proton/Wine prefix (`steamapps/compatdata/<id>/pfx`), found
    /// from the configured game folder. Linux only, and only subtrees named in
    /// [`crate::plugins::jail::PREFIX_ALLOWED`]; see [`crate::plugins::prefix`].
    Prefix,
    /// The layer being built by an install manifest: its `files/`, `work/`
    /// and `meta/`. Install manifests only; a plugin naming it is refused.
    Layer,
    /// The installed runtime an install manifest runs, read-only. Install
    /// manifests only.
    Runtime,
}

impl FsRoot {
    /// The name a manifest writes.
    pub fn name(self) -> &'static str {
        match self {
            Self::GameDir => "gameDir",
            Self::PluginData => "pluginData",
            Self::Downloads => "downloads",
            Self::Prefix => "prefix",
            Self::Layer => "layer",
            Self::Runtime => "runtime",
        }
    }
}

/// One subtree of one root the plugin may touch.
///
/// Several grants may name the same root — `{gameDir, "mods", write}` and
/// `{gameDir, "config", write}` — and they compose as a union of permitted
/// subtrees rather than merging. A `PathRef` is always resolved against the
/// ROOT and then checked against these, so declaring narrow access stays narrow
/// however many grants there are and in whatever order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FsGrant {
    pub root: FsRoot,
    /// Subdirectory under the root, e.g. `mods`. Empty means the root itself.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub write: bool,
}

// ------------------------------------------------------------------ Installer

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Installer {
    /// Which content kinds this installer handles: `mod`, `asset`, or both.
    #[serde(default)]
    pub kinds: Vec<String>,
    pub install: Vec<Step>,
    #[serde(default)]
    pub uninstall: Vec<Step>,
}

/// One action in an install or uninstall plan.
///
/// Note what is NOT here: no shell, no process spawn, no environment access, no
/// arbitrary path. `Download` is the only step that touches the network and it
/// is bounded by the manifest's host allow-list; every path in every other step
/// is a *relative* path resolved against a granted root.
/*
 * `rename_all_fields` as well as `rename_all`: the former renames each
 * variant's FIELDS, the latter only the variant tags. Without it a manifest
 * would have to write `max_bytes`, which no other key in the format does.
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Step {
    /// Fetch a file into the jail.
    ///
    /// `url` may contain `{}`-placeholders filled from the install context
    /// (the release file's URL, the item id). It is re-checked against the
    /// allow-list AFTER substitution, so a placeholder cannot smuggle a host in.
    Download {
        /// Empty only in an install manifest that names an `artifact`, whose
        /// URL, mirrors and digest the app fills from the bindings.
        #[serde(default)]
        url: String,
        /// Install manifests only: the binding key the app resolves, also the
        /// name of `{artifact.<key>}` for later steps.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact: Option<String>,
        to: PathRef,
        /// Expected SHA-256, when the plugin author knows it. A mismatch aborts
        /// the whole plan.
        #[serde(default)]
        sha256: Option<String>,
        #[serde(default)]
        max_bytes: Option<u64>,
        /// Alternative URLs for the same file, tried in order after `url`
        /// fails on the network or arrives with the wrong digest.
        ///
        /// Each passes the same https and `permissions.net` checks as `url`,
        /// and `sha256` is REQUIRED (64 hex digits) when any are listed.
        #[serde(default)]
        mirrors: Vec<String>,
    },

    /// Unpack a `.zip`, `.tar`, `.tar.gz` inside the jail.
    Extract {
        from: PathRef,
        to: PathRef,
        /// Drop this many leading path components from each entry.
        #[serde(default)]
        strip: u8,
        /// After `strip`, drop one more leading folder when every file in the
        /// archive shares the same one (`mod-1.2/...`), so a wrapper folder an
        /// author did or did not add lands in the same place. Archives whose
        /// files already sit at the top, or under several folders, are left
        /// as they are; a macOS `__MACOSX/` sibling is dropped.
        #[serde(default)]
        strip_single_root: bool,
        /// Only extract entries whose path starts with one of these prefixes.
        #[serde(default)]
        include: Vec<String>,
    },

    Copy {
        from: PathRef,
        to: PathRef,
    },

    Move {
        from: PathRef,
        to: PathRef,
    },

    Mkdir {
        path: PathRef,
    },

    /// Delete a file, or a directory the plan itself created.
    Remove {
        path: PathRef,
    },

    /// Write a UTF-8 file. Content may contain context placeholders.
    WriteText {
        path: PathRef,
        content: String,
        /// Leave an existing file alone (a config the user may have edited).
        #[serde(default, skip_serializing_if = "is_false")]
        if_missing: bool,
    },

    /// Set one value in a JSON file, creating it if absent.
    ///
    /// A whole-file write is what a plugin would otherwise have to do to change
    /// one setting, and that silently discards everything else in the user's
    /// config. `pointer` is an RFC 6901 JSON Pointer.
    PatchJson {
        path: PathRef,
        pointer: String,
        value: serde_json::Value,
    },

    /// Install manifests only (see `docs/loader-manifests.md`): run the
    /// loader's own installer from a runtime, with a cleared environment,
    /// `work/` as the cwd and a timeout that kills the whole process tree.
    /// Never allowed in a plugin; [`check_plugin_steps`] refuses it.
    RunInstaller {
        /// `{runtime.<kind>}` or `{artifact.<key>}`, nothing else.
        program: String,
        #[serde(default)]
        args: Vec<String>,
        /// Always `{layerWork}`.
        cwd: String,
        /// Default 900; 0 is no timeout.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
        #[serde(default)]
        network: bool,
        /// Paths under `layer` that must exist after exit 0.
        #[serde(default)]
        expect_files: Vec<PathRef>,
    },

    /// Install manifests only: fetch every entry of a JSON list (Mojang's
    /// `libraries`, say) that an earlier pinned `download` brought in.
    DownloadList {
        list: PathRef,
        /// RFC 6901 pointer to the array; `""` is the document itself.
        pointer: String,
        /// Pointers, relative to one entry, to its URL, path and digests.
        url_field: String,
        path_field: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha1_field: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256_field: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size_field: Option<String>,
        to: PathRef,
        /// Most entries; default 10000, 0 is unlimited.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<u64>,
    },

    /// Install manifests only: merge `overlay` into `base`, writing `to`.
    /// A deep merge where the overlay wins, except at the pointers in `rules`.
    MergeJson {
        base: PathRef,
        overlay: PathRef,
        to: PathRef,
        #[serde(default)]
        rules: Vec<MergeRule>,
    },

    /// Install manifests only: fail unless the file exists (and, with
    /// `sha256`, has exactly that digest).
    RequireFile {
        path: PathRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// How `mergeJson` combines the two values at one pointer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MergeRule {
    pub pointer: String,
    pub mode: MergeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeMode {
    /// The overlay's value, whole.
    Replace,
    /// Base array, then overlay array.
    Append,
    /// Overlay array, then base array.
    Prepend,
    /// Base array, then each overlay item not already in it.
    Union,
}

impl Step {
    /// The `action` a manifest writes.
    pub fn action_name(&self) -> &'static str {
        match self {
            Self::Download { .. } => "download",
            Self::Extract { .. } => "extract",
            Self::Copy { .. } => "copy",
            Self::Move { .. } => "move",
            Self::Mkdir { .. } => "mkdir",
            Self::Remove { .. } => "remove",
            Self::WriteText { .. } => "writeText",
            Self::PatchJson { .. } => "patchJson",
            Self::RunInstaller { .. } => "runInstaller",
            Self::DownloadList { .. } => "downloadList",
            Self::MergeJson { .. } => "mergeJson",
            Self::RequireFile { .. } => "requireFile",
        }
    }

    /// Steps only an install manifest may use.
    pub fn is_install_only(&self) -> bool {
        matches!(
            self,
            Self::RunInstaller { .. }
                | Self::DownloadList { .. }
                | Self::MergeJson { .. }
                | Self::RequireFile { .. }
        )
    }

    /// Every path the step names.
    pub fn paths(&self) -> Vec<&PathRef> {
        match self {
            Self::Download { to, .. } => vec![to],
            Self::Extract { from, to, .. } | Self::Copy { from, to } | Self::Move { from, to } => {
                vec![from, to]
            }
            Self::Mkdir { path }
            | Self::Remove { path }
            | Self::WriteText { path, .. }
            | Self::PatchJson { path, .. }
            | Self::RequireFile { path, .. } => vec![path],
            Self::RunInstaller { expect_files, .. } => expect_files.iter().collect(),
            Self::DownloadList { list, to, .. } => vec![list, to],
            Self::MergeJson {
                base, overlay, to, ..
            } => vec![base, overlay, to],
        }
    }
}

/// Refuses what only an install manifest may say: the four install-only
/// steps, the `layer` and `runtime` roots (in a step or a grant), and a
/// `download` by `artifact`. Every plugin step list passes through this —
/// [`Manifest::validate`] and the app-plugin `manage` block alike — so a
/// plugin can never reach the step that runs a program.
pub fn check_plugin_steps(steps: &[Step], grants: &[FsGrant]) -> AppResult<()> {
    for grant in grants {
        if matches!(grant.root, FsRoot::Layer | FsRoot::Runtime) {
            return Err(AppError::invalid(format!(
                "The {} root is not allowed in plugins.",
                grant.root.name()
            )));
        }
    }

    for step in steps {
        if step.is_install_only() {
            return Err(AppError::invalid(format!(
                "{} is not allowed in plugins",
                step.action_name()
            )));
        }

        for path in step.paths() {
            if matches!(path.root, FsRoot::Layer | FsRoot::Runtime) {
                return Err(AppError::invalid(format!(
                    "The {} root is not allowed in plugins.",
                    path.root.name()
                )));
            }
        }

        if let Step::Download { url, artifact, .. } = step {
            if artifact.is_some() {
                return Err(AppError::invalid(
                    "download.artifact is not allowed in plugins",
                ));
            }

            if url.trim().is_empty() {
                return Err(AppError::invalid("A download needs a url."));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------- Install manifests

/// When an install step applies (`docs/loader-manifests.md`, "Steps"). Every
/// field that is present must match; an absent field matches anything.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct When {
    /// `client` or `server`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    /// `windows`, `linux`, `macos`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
    /// A consent key the user must have granted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent: Option<String>,
}

/// What a `when` is matched against.
#[derive(Debug, Clone, Default)]
pub struct WhenContext {
    pub side: String,
    pub platform: String,
    pub consents: std::collections::BTreeSet<String>,
}

impl When {
    pub fn matches(&self, ctx: &WhenContext) -> bool {
        self.side.as_ref().is_none_or(|s| *s == ctx.side)
            && self
                .platforms
                .as_ref()
                .is_none_or(|p| p.contains(&ctx.platform))
            && self
                .consent
                .as_ref()
                .is_none_or(|c| ctx.consents.contains(c))
    }
}

/// One step of an install manifest's `install` list: a [`Step`] plus the
/// fields every install step may carry (`when`, `label`, a placeholder note).
#[derive(Debug, Clone)]
pub struct InstallStep {
    pub step: Step,
    pub when: Option<When>,
    pub label: Option<String>,
}

impl<'de> Deserialize<'de> for InstallStep {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        let mut value = serde_json::Map::<String, serde_json::Value>::deserialize(d)?;
        let when = value
            .remove("when")
            .map(serde_json::from_value::<When>)
            .transpose()
            .map_err(D::Error::custom)?;
        let label = match value.remove("label") {
            None => None,
            Some(serde_json::Value::String(s)) => Some(s),
            Some(_) => return Err(D::Error::custom("label must be a string")),
        };
        value.remove("x-tmc-placeholder");
        let step =
            serde_json::from_value(serde_json::Value::Object(value)).map_err(D::Error::custom)?;

        Ok(Self { step, when, label })
    }
}

/// Parses and checks an install manifest's `install` list. The schema and
/// lint pass ([`crate::layers::lint`]) run first in the real flow; this is
/// the executor's own refusal, so a step list that skipped them still cannot
/// name a root an install manifest has no business with.
pub fn parse_install_steps(value: &serde_json::Value) -> AppResult<Vec<InstallStep>> {
    let steps: Vec<InstallStep> = serde_json::from_value(value.clone())
        .map_err(|e| AppError::invalid(format!("Install steps are invalid: {e}")))?;

    check_install_steps(&steps)?;

    Ok(steps)
}

/// `gameDir`, `pluginData` and `prefix` are not roots of an install
/// manifest; a download needs exactly one of `artifact` and `url`, and a
/// `url` needs a pinned sha256.
pub fn check_install_steps(steps: &[InstallStep]) -> AppResult<()> {
    for (index, s) in steps.iter().enumerate() {
        for path in s.step.paths() {
            if !matches!(
                path.root,
                FsRoot::Layer | FsRoot::Downloads | FsRoot::Runtime
            ) {
                return Err(AppError::invalid(format!(
                    "Step {index}: {} is not valid in an install manifest.",
                    path.root.name()
                )));
            }
        }

        if let Step::Download {
            url,
            artifact,
            sha256,
            ..
        } = &s.step
        {
            let has_url = !url.trim().is_empty();

            if has_url == artifact.is_some() {
                return Err(AppError::invalid(format!(
                    "Step {index}: a download names exactly one of artifact and url."
                )));
            }

            if has_url
                && !sha256
                    .as_deref()
                    .is_some_and(|s| crate::plugins::steps::is_sha256_hex(s.trim()))
            {
                return Err(AppError::invalid(format!(
                    "Step {index}: an install download needs a pinned sha256."
                )));
            }
        }
    }

    Ok(())
}

/// A path, always relative and always anchored to a granted root.
///
/// There is no variant for an absolute path. That is not an oversight — the
/// type simply cannot express one, so no amount of manifest creativity produces
/// a write outside a root the user granted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathRef {
    pub root: FsRoot,
    /// Relative to the ROOT — not to any one `FsGrant` — and `/`-separated.
    /// So a plugin granted `{gameDir, "mods"}` writes to
    /// `{ root: gameDir, path: "mods/foo.jar" }`. Validated by
    /// [`crate::plugins::jail::Jail::resolve`].
    pub path: String,
}

// -------------------------------------------------------------- Server query

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerQuery {
    /// `udp` or `tcp`. Only `udp` is implemented; `tcp` is reserved.
    pub protocol: String,
    /// Which port to use: `game`, `query`, or a fixed number.
    #[serde(default)]
    pub port: PortSource,
    /// Hex request payload. May contain `{challenge}` for two-stage protocols.
    pub request_hex: String,
    #[serde(default = "default_query_timeout")]
    pub timeout_ms: u64,
    /// How to read the reply.
    pub parse: Vec<Field>,
}

fn default_query_timeout() -> u64 {
    1_500
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PortSource {
    #[default]
    Query,
    Game,
    Fixed(u16),
}

/// One value to pull out of a query response.
///
/// A tiny read-only grammar rather than a regex or an expression language: a
/// parser a plugin can drive is a parser a plugin can hang the app with, and
/// every variant here consumes a bounded number of bytes from a bounded buffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Field {
    /// Skip forward.
    Skip {
        bytes: u32,
    },
    U8 {
        name: String,
    },
    U16Le {
        name: String,
    },
    U16Be {
        name: String,
    },
    U32Le {
        name: String,
    },
    U32Be {
        name: String,
    },
    /// NUL-terminated string.
    CString {
        name: String,
    },
    /// Length-prefixed string, prefix is a single byte.
    PString {
        name: String,
    },
    /// Assert the next bytes equal this hex value, else the parse fails.
    Magic {
        hex: String,
    },
}

// --------------------------------------------------------------------- Theme

/// A theme is a token map, not a stylesheet.
///
/// Arbitrary CSS would let a plugin load a remote font or background image and
/// turn every app launch into a beacon to the author's server — and would let
/// it reposition or hide UI, including the buttons that uninstall it. A fixed
/// list of custom properties with strictly-validated values can do neither.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Theme {
    pub label: String,
    /// `light` or `dark` — which base the tokens are overriding.
    #[serde(default)]
    pub base: Option<String>,
    /// `--token` → value. Both sides are validated; see
    /// [`crate::plugins::theme`].
    pub tokens: BTreeMap<String, String>,
}

// ---------------------------------------------------------------- Validation

/// Reverse-DNS-ish, and safe as a single path component on every platform.
///
/// Strict because this string becomes a directory name: no separators, no
/// dots-only names, no Windows reserved device names, no leading/trailing dot
/// or space (which Windows silently strips, producing a different directory
/// than the one that was checked).
pub fn is_valid_plugin_id(id: &str) -> bool {
    if id.len() < 3 || id.len() > 96 {
        return false;
    }

    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-' || c == '_')
    {
        return false;
    }

    if id.starts_with('.') || id.ends_with('.') || id.contains("..") {
        return false;
    }

    const RESERVED: [&str; 6] = ["con", "prn", "aux", "nul", "com1", "lpt1"];

    !RESERVED.contains(&id)
}

/// A host entry: `example.com` or `*.example.com`. Nothing else.
fn is_valid_host_pattern(pattern: &str) -> bool {
    let host = pattern.strip_prefix("*.").unwrap_or(pattern);

    // A bare `*` or an empty label after the wildcard would match everything.
    if host.is_empty() || host == "*" || host.contains('*') {
        return false;
    }

    if host.len() > 253 || !host.contains('.') {
        return false;
    }

    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

impl Manifest {
    /// Parse and validate. A manifest that fails here is never stored, so an
    /// installed plugin is one that has already passed every check below.
    pub fn parse(raw: &str) -> AppResult<Self> {
        if raw.len() > 256 * 1024 {
            return Err(AppError::invalid("Plugin manifest is too large."));
        }

        // `deny_unknown_fields` throughout: a manifest carrying a key we do not
        // understand is either from a newer app version or is trying something,
        // and both deserve a refusal rather than a silent ignore.
        let manifest: Manifest = serde_json::from_str(raw)
            .map_err(|e| AppError::invalid(format!("Plugin manifest is invalid: {e}")))?;

        manifest.validate()?;

        Ok(manifest)
    }

    fn validate(&self) -> AppResult<()> {
        if self.manifest_version != MANIFEST_VERSION {
            return Err(AppError::invalid(format!(
                "Plugin manifest version {} is not supported by this app version.",
                self.manifest_version
            )));
        }

        if !is_valid_plugin_id(&self.id) {
            return Err(AppError::invalid(
                "Plugin id must be lowercase letters, digits, '.', '-' or '_'.",
            ));
        }

        for field in [&self.name, &self.version, &self.author] {
            if field.trim().is_empty() || field.len() > 128 {
                return Err(AppError::invalid(
                    "Plugin name, version and author are required.",
                ));
            }
        }

        if self.installer.is_none() && self.server_query.is_none() && self.theme.is_none() {
            return Err(AppError::invalid("Plugin declares no capability."));
        }

        for host in &self.permissions.net {
            if !is_valid_host_pattern(host) {
                return Err(AppError::invalid(format!(
                    "'{host}' is not a valid host. Use 'example.com' or '*.example.com'."
                )));
            }
        }

        if let Some(installer) = &self.installer {
            if installer.install.len() > 128 || installer.uninstall.len() > 128 {
                return Err(AppError::invalid("Plugin has too many steps."));
            }

            check_plugin_steps(&installer.install, &self.permissions.fs)?;
            check_plugin_steps(&installer.uninstall, &[])?;

            for step in installer.install.iter().chain(installer.uninstall.iter()) {
                if let Step::Download {
                    mirrors, sha256, ..
                } = step
                {
                    if mirrors.len() > MAX_MIRRORS {
                        return Err(AppError::invalid("A download lists too many mirrors."));
                    }

                    let pinned = sha256
                        .as_deref()
                        .is_some_and(|s| crate::plugins::steps::is_sha256_hex(s.trim()));

                    if !mirrors.is_empty() && !pinned {
                        return Err(AppError::invalid(
                            "A download with mirrors needs a pinned 64-hex sha256.",
                        ));
                    }
                }
            }

            /*
             * A plan with no write grant can only fail, loudly, halfway
             * through. Catching it at install time means the user is told
             * "this plugin is broken" rather than "installing Foo failed".
             */
            if !installer.install.is_empty() && !self.permissions.fs.iter().any(|g| g.write) {
                return Err(AppError::invalid(
                    "Plugin has install steps but no writable filesystem permission.",
                ));
            }
        }

        if let Some(q) = &self.server_query {
            if q.protocol != "udp" {
                return Err(AppError::invalid(
                    "Only 'udp' server queries are supported.",
                ));
            }

            if !self.permissions.query {
                return Err(AppError::invalid(
                    "Plugin declares a server query but not the 'query' permission.",
                ));
            }

            if q.parse.len() > 64 {
                return Err(AppError::invalid("Query parser has too many fields."));
            }
        }

        if let Some(theme) = &self.theme {
            crate::plugins::theme::validate(theme)?;
        }

        Ok(())
    }

    /// Whether `host` is covered by the net allow-list.
    pub fn allows_host(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();

        self.permissions.net.iter().any(|pattern| {
            let pattern = pattern.to_ascii_lowercase();

            match pattern.strip_prefix("*.") {
                // `*.example.com` covers `a.example.com` but NOT `example.com`
                // itself, and not `notexample.com` — the dot is part of the
                // suffix on purpose.
                Some(suffix) => host.ends_with(&format!(".{suffix}")),
                None => host == pattern,
            }
        })
    }

    /// The identity the user approves.
    ///
    /// Over the manifest's canonical bytes, not over the bundle: what the user
    /// is consenting to is the declared permissions and steps. Re-serialising
    /// through serde first means whitespace and key order cannot change the
    /// hash, so a re-download of the same plugin does not re-prompt.
    pub fn fingerprint(&self) -> String {
        let canonical = serde_json::to_vec(self).unwrap_or_default();

        hex::encode(Sha256::digest(canonical))
    }

    /// A plain-language list of what approving this plugin allows, for the
    /// consent dialog. The dialog shows exactly this — no summarising, no
    /// "and 3 more".
    pub fn permission_summary(&self) -> Vec<String> {
        let mut out = Vec::new();

        for grant in &self.permissions.fs {
            let where_ = match grant.root {
                FsRoot::GameDir => "the game's install folder",
                FsRoot::PluginData => "its own private folder",
                FsRoot::Downloads => "your downloads folder",
                FsRoot::Prefix => "the game's Wine/Proton prefix (Linux only)",
                // Refused by `check_plugin_steps`; named rather than hidden.
                FsRoot::Layer => "a loader layer",
                FsRoot::Runtime => "a runtime",
            };

            let sub = if grant.path.is_empty() {
                String::new()
            } else {
                format!(" ({})", grant.path)
            };

            out.push(format!(
                "{} files in {where_}{sub}",
                if grant.write {
                    "Read and write"
                } else {
                    "Read"
                }
            ));
        }

        for host in &self.permissions.net {
            out.push(format!("Download files from {host}"));
        }

        if self.permissions.query {
            out.push("Send queries to game servers you view".into());
        }

        if self.theme.is_some() {
            out.push("Change the app's colours".into());
        }

        /*
         * Named as a READ, because that is all it is and the difference is the
         * whole reason somebody would agree to it. A manager descriptor lists
         * directories under a base the app resolved; it writes nothing, and
         * importing from what it found is a separate click on a separate
         * screen.
         */
        if let Some(manager) = &self.manager {
            out.push(format!(
                "Look for installed mods in {}'s own folders",
                manager.label
            ));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_ids_reject_path_tricks() {
        for bad in [
            "..",
            "a/b",
            "a\\b",
            ".hidden",
            "trailing.",
            "a..b",
            "con",
            "UPPER",
            "x",
        ] {
            assert!(!is_valid_plugin_id(bad), "{bad} should be rejected");
        }

        assert!(is_valid_plugin_id("com.example.installer"));
        assert!(is_valid_plugin_id("my-plugin_2"));
    }

    #[test]
    fn host_patterns_cannot_widen_to_everything() {
        for bad in ["*", "*.", "*.com*", "", "localhost", "a..b.com"] {
            assert!(!is_valid_host_pattern(bad), "{bad} should be rejected");
        }

        assert!(is_valid_host_pattern("example.com"));
        assert!(is_valid_host_pattern("*.example.com"));
    }

    /// The examples are the documentation. A manifest field renamed without
    /// updating them would ship a reference that does not load.
    #[test]
    fn the_shipped_examples_all_validate() {
        // `core/` → `src-tauri/` → repo root.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins");

        let entries = std::fs::read_dir(&root)
            .unwrap_or_else(|e| panic!("no examples at {}: {e}", root.display()));

        let mut checked = 0;

        for entry in entries.flatten() {
            let path = entry.path().join(MANIFEST_FILE);

            if !path.exists() {
                continue;
            }

            let raw = std::fs::read_to_string(&path).expect("readable");

            Manifest::parse(&raw).unwrap_or_else(|e| panic!("{} is invalid: {e}", path.display()));

            checked += 1;
        }

        assert!(
            checked >= 3,
            "expected the example plugins, found {checked}"
        );
    }

    #[test]
    fn wildcard_matches_subdomains_only() {
        let m = Manifest {
            manifest_version: 1,
            id: "a.b".into(),
            name: "t".into(),
            version: "1".into(),
            author: "t".into(),
            description: None,
            homepage: None,
            apps: vec![],
            permissions: Permissions {
                net: vec!["*.example.com".into()],
                ..Default::default()
            },
            installer: None,
            server_query: None,
            theme: None,
            manager: None,
        };

        assert!(m.allows_host("cdn.example.com"));
        assert!(!m.allows_host("example.com"));
        assert!(!m.allows_host("notexample.com"));
        assert!(!m.allows_host("example.com.evil.net"));
    }
}
