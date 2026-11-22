//! The catalog's wire shapes (W5, `/api/app/v1/apps/:id/...`) and what the
//! webview is handed.
//!
//! The field names are the website contract's (`CatalogWireSchema` and its
//! neighbours in `src/lib/api/contract.ts`), and every wire struct refuses
//! unknown fields: the catalog decides what gets installed, so a response that
//! is not exactly the agreed shape is not trusted, cached or shown. The
//! compiled-in snapshot (`manifests/<app>/catalog.snapshot.json`) uses the same
//! entry shapes, so one set of structs reads both.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A release channel. `STABLE` is the default the server applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Channel {
    Stable,
    Preview,
    Experimental,
    Legacy,
}

impl Channel {
    pub const ALL: [Channel; 4] = [
        Channel::Stable,
        Channel::Preview,
        Channel::Experimental,
        Channel::Legacy,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "STABLE",
            Channel::Preview => "PREVIEW",
            Channel::Experimental => "EXPERIMENTAL",
            Channel::Legacy => "LEGACY",
        }
    }
}

/// `{at, note, withdrawn}` on a deprecated (or withdrawn) row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Deprecation {
    pub at: String,
    pub note: Option<String>,
    pub withdrawn: bool,
}

/// The game a response is about. The snapshot names only the slug.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppRef {
    #[serde(default)]
    pub id: Option<i64>,
    pub slug: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameVersion {
    pub version_id: String,
    pub label: Option<String>,
    pub channel: Channel,
    pub channel_label: Option<String>,
    pub sort_key: i64,
    pub released_at: Option<String>,
    pub deprecated: Option<Deprecation>,
    pub managed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameLoader {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    /// `CLIENT`, `SERVER` or `BOTH`.
    pub sides: String,
    pub online_note: Option<String>,
    pub position: i64,
    pub custom_versions: bool,
    pub managed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoaderVersion {
    pub version: String,
    pub channel: Channel,
    pub sort_key: i64,
    pub recommended: bool,
    pub custom: bool,
    pub deprecated: Option<Deprecation>,
}

/// `{items, nextCursor}`; the cursor is opaque.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

/// `GET /apps/:id/catalog`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogWire {
    pub app: AppRef,
    pub loaders: Vec<GameLoader>,
    pub versions: Page<GameVersion>,
    pub generated_at: String,
}

/// `GET /apps/:id/loaders/:key/versions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoaderVersionList {
    pub loader: String,
    pub game_version: Option<String>,
    pub items: Vec<LoaderVersion>,
    pub next_cursor: Option<String>,
}

/// One resolved download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactWire {
    pub url: String,
    pub mirrors: Vec<String>,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayerBindings {
    pub vars: BTreeMap<String, String>,
    pub artifacts: BTreeMap<String, ArtifactWire>,
}

/// One layer of a served plan: the manifest itself and what fills it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanLayerWire {
    pub manifest_key: String,
    pub revision: u64,
    pub sha256: String,
    pub exec: bool,
    /// Detached Ed25519, base64. Only exec layers carry one.
    pub signature: Option<String>,
    pub manifest: Map<String, Value>,
    pub bindings: LayerBindings,
}

/// `GET /runtimes/:kind/:major`, and each entry of a plan's `runtimes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeWire {
    pub kind: String,
    pub major: String,
    pub vendor_label: String,
    pub entry: String,
    pub strip_single_root: bool,
    pub artifact: ArtifactWire,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanRequestWire {
    pub game_version: String,
    pub loader: Option<String>,
    pub loader_version: Option<String>,
    pub platform: String,
    pub side: String,
}

/// `GET /apps/:id/install-plan`: the resolved, unexecuted plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallPlanWire {
    pub app: AppRef,
    pub request: PlanRequestWire,
    pub layers: Vec<PlanLayerWire>,
    pub runtimes: Vec<RuntimeWire>,
    pub deprecated: Option<Deprecation>,
    pub plan_version: u32,
}

/// The parts of `catalog.snapshot.json` the pickers read. The snapshot also
/// carries `versionBindings` and full `builds` rows, which only plan
/// resolution reads (`layers::plan`), so this struct does not refuse them.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub app: AppRef,
    #[serde(default)]
    pub generated_at: Option<String>,
    pub loaders: Vec<GameLoader>,
    pub versions: Page<GameVersion>,
    #[serde(default)]
    pub loader_versions: BTreeMap<String, Vec<LoaderVersion>>,
    #[serde(default)]
    pub builds: Vec<SnapshotBuild>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotBuild {
    pub loader: String,
    pub loader_version: String,
    pub game_version: String,
}

/// Where a list shown to the user came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Fetched just now.
    Remote,
    /// The library database's copy, inside its TTL or confirmed by a 304, or
    /// stale because the network failed (`stale: true`).
    Cache,
    /// The snapshot compiled into this build: there was no cached copy and
    /// the network failed.
    Compiled,
}

/// Where a value came from and how old it is; flattened into each view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub source: Source,
    /// Served from the cache after the network failed: the picker says
    /// "Offline: list from <fetchedAt>".
    pub stale: bool,
    /// Unix milliseconds of the last fetch or revalidation; `None` for the
    /// compiled snapshot.
    pub fetched_at: Option<i64>,
}

/// `catalog_get`: the picker in one call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogView {
    pub app: AppRef,
    pub loaders: Vec<GameLoader>,
    pub versions: Page<GameVersion>,
    pub generated_at: Option<String>,
    #[serde(flatten)]
    pub provenance: Provenance,
}

/// `catalog_loader_versions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderVersionsView {
    pub loader: String,
    pub game_version: Option<String>,
    pub items: Vec<LoaderVersion>,
    pub next_cursor: Option<String>,
    #[serde(flatten)]
    pub provenance: Provenance,
}
