//! Install manifests and the layers built from them (docs/loader-manifests.md,
//! docs/loaders-and-versions.md "Deploy consumption").
//!
//! ```text
//!   plan     — catalog + manifests  → InstallPlan (resolve, trust, hashes)
//!   consent  — exec, runtime downloads and manifest consents, per plan hash
//!   runtime  — the Java (or other) runtimes an install step runs
//!   build    — one layer per (manifest, bindings), reused across sandboxes
//!   checks   — what a built layer may contain
//! ```
//!
//! The deploy (library/deploy.rs) mounts each layer as a lowest-priority
//! pseudo-mod and applies the plan's JSON patches after the merge.

pub mod build;
pub mod canonical;
pub mod checks;
pub mod consent;
pub mod lint;
pub mod placeholders;
pub mod plan;
pub mod runtime;
pub mod schema;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// What a sandbox's last deploy installed: the `sandboxes.install_state`
/// column, shown in the UI and carried by share and export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallState {
    pub plan_hash: String,
    pub layers: Vec<InstallStateLayer>,
    /// `launch.vars` of the plan (`launchVersionId`), read by the launch.
    #[serde(default)]
    pub launch_vars: BTreeMap<String, String>,
    pub deployed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallStateLayer {
    pub manifest_key: String,
    pub revision: u32,
    pub layer_hash: String,
}
