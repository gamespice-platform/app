//! A served install plan (`GET /apps/:id/install-plan`) as a [`PlanSource`],
//! so the same resolver, lint and signature check that the compiled manifests
//! go through decide what a fetched plan installs.
//!
//! The served plan is already resolved to one game version and at most one
//! loader build, so the catalog synthesised here holds exactly that: one
//! managed version with the version layer's bindings, and, with a loader, one
//! managed loader, one version of it and one build naming the loader layer.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};
use crate::layers::canonical;
use crate::layers::lint::{verify_signature, ExecKey};
use crate::layers::plan::{
    ArtifactBinding, ManifestEntry, PlanRequest, PlanSource, RuntimeOffer, Trust,
};

use super::types::{InstallPlanWire, PlanLayerWire};

/// The loader, or `None` for no loader (absent, empty or `vanilla`).
fn vanilla(loader: Option<&str>) -> Option<&str> {
    loader.filter(|l| !l.is_empty() && !l.eq_ignore_ascii_case("vanilla"))
}

fn refuse(why: impl std::fmt::Display) -> AppError {
    AppError::invalid(format!(
        "The install plan the server sent cannot be used: {why}"
    ))
}

/// The plan as a source for `req`, trusted as `Official`. Refused when it
/// answers a different question than `req` asked, when a layer's manifest is
/// not the one its key and hash name, or when the version layer is missing;
/// the exec signature is checked later, by the resolver's lint.
pub fn plan_source(
    wire: &InstallPlanWire,
    req: &PlanRequest,
    keys: &[ExecKey],
) -> AppResult<PlanSource> {
    if wire.plan_version != 1 {
        return Err(refuse(format!(
            "plan version {} is newer than this app",
            wire.plan_version
        )));
    }
    let asked = &wire.request;
    if wire.app.slug != req.app_slug
        || asked.game_version != req.game_version
        || vanilla(asked.loader.as_deref()) != vanilla(req.loader.as_deref())
        || (vanilla(req.loader.as_deref()).is_some() && asked.loader_version != req.loader_version)
        || asked.platform != req.platform
        || asked.side != req.side
    {
        return Err(refuse("it answers a different request"));
    }

    let slug = &wire.app.slug;
    let version_key = format!("{slug}/version");
    let mut manifests = BTreeMap::new();
    let mut version_layer: Option<&PlanLayerWire> = None;
    let mut loader_layer: Option<&PlanLayerWire> = None;

    for layer in &wire.layers {
        let doc = Value::Object(layer.manifest.clone());
        if doc.get("id").and_then(Value::as_str) != Some(layer.manifest_key.as_str()) {
            return Err(refuse(format!(
                "layer {} carries another manifest",
                layer.manifest_key
            )));
        }
        if doc.get("revision").and_then(Value::as_u64) != Some(layer.revision) {
            return Err(refuse(format!(
                "layer {} names another revision",
                layer.manifest_key
            )));
        }
        let bytes = canonical::canonical_bytes(&doc).map_err(|e| refuse(e.0))?;
        let hash = hex::encode(Sha256::digest(&bytes));
        if !hash.eq_ignore_ascii_case(&layer.sha256) {
            return Err(refuse(format!(
                "layer {} does not match its hash",
                layer.manifest_key
            )));
        }
        let text = canonical::pretty_string(&doc).map_err(|e| refuse(e.0))?;
        let sig = layer.signature.as_deref().map(|s| signature(&doc, s, keys));
        manifests.insert(layer.manifest_key.clone(), ManifestEntry { doc, text, sig });

        if layer.manifest_key == version_key {
            version_layer = Some(layer);
        } else if loader_layer.replace(layer).is_some() {
            return Err(refuse("it has more than one loader layer"));
        }
    }

    let version_layer =
        version_layer.ok_or_else(|| refuse(format!("it has no {version_key} layer")))?;
    let gv = &asked.game_version;
    let mut catalog = json!({
        "app": { "slug": slug },
        "versions": { "items": [{
            "versionId": gv,
            "managed": true,
            "deprecated": wire.deprecated,
        }], "nextCursor": null },
        "versionBindings": { gv.as_str(): version_layer.bindings },
        "loaders": [],
        "loaderVersions": {},
        "builds": [],
    });

    match (vanilla(asked.loader.as_deref()), loader_layer) {
        (Some(loader), Some(layer)) => {
            let lv = asked.loader_version.clone().unwrap_or_default();
            let name = layer
                .manifest
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(loader);
            catalog["loaders"] =
                json!([{ "key": loader, "name": name, "managed": true, "customVersions": false }]);
            catalog["loaderVersions"] =
                json!({ loader: [{ "version": lv, "custom": false, "deprecated": null }] });
            catalog["builds"] = json!([{
                "loader": loader,
                "loaderVersion": lv,
                "gameVersion": gv,
                "manifestKey": layer.manifest_key,
                "bindings": layer.bindings,
                "revision": layer.revision,
            }]);
        }
        (None, None) => {}
        _ => return Err(refuse("its layers do not match its loader")),
    }

    let runtimes = wire
        .runtimes
        .iter()
        .map(|r| RuntimeOffer {
            kind: r.kind.clone(),
            major: r.major.clone(),
            platform: asked.platform.clone(),
            vendor_label: Some(r.vendor_label.clone()),
            entry: r.entry.clone(),
            strip_single_root: r.strip_single_root,
            artifact: ArtifactBinding {
                url: r.artifact.url.clone(),
                mirrors: r.artifact.mirrors.clone(),
                sha256: r.artifact.sha256.clone(),
                size: Some(r.artifact.size),
            },
        })
        .collect();

    Ok(PlanSource {
        trust: Trust::Official,
        manifests,
        keys: keys.to_vec(),
        catalogs: BTreeMap::from([(slug.clone(), catalog)]),
        runtimes,
    })
}

/// The wire carries the bare signature; the lint wants `{alg, keyId,
/// signature}`. The key that verifies it is named, or, when none does, the
/// first one, so the resolver refuses the layer as `signature-invalid` rather
/// than as malformed.
fn signature(doc: &Value, signature: &str, keys: &[ExecKey]) -> Value {
    let with = |key_id: &str| json!({ "alg": "ed25519", "keyId": key_id, "signature": signature });
    keys.iter()
        .map(|k| with(&k.key_id))
        .find(|sig| verify_signature(doc, sig, keys).is_none())
        .unwrap_or_else(|| with(keys.first().map(|k| k.key_id.as_str()).unwrap_or("")))
}
