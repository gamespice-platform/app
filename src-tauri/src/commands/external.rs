//! Mods from other sites: Modrinth, Thunderstore, GitHub, direct links.
//!
//! Same rule as `commands/import.rs`: no command takes a filesystem path.
//! `external_inspect_path` takes a vault token minted when the user dropped
//! the file. Links that arrive from the OS are routed by `lib.rs` to
//! [`tmc_core::external::pending`] and become a confirm screen; the install
//! commands here run only because the user pressed the button or pasted.

use serde::{Deserialize, Serialize};
use tauri::State;

use tmc_core::error::{AppError, AppResult};
use tmc_core::external::install::{self, AddOutcome, InstallEnv, Target};
use tmc_core::external::matching::{self, Candidate, GameRef, SandboxRef};
use tmc_core::external::meta::{self, Inspection};
use tmc_core::external::pending::{self, Pending};
use tmc_core::external::provider::{HttpFetch, ResolveCtx};
use tmc_core::external::scheme::{self, Os, SchemeStatus, SystemRunner};
use tmc_core::external::tree::Source;
use tmc_core::external::{
    parse_input, CredentialState, ExternalRef, ProviderAvailability, ProviderId, Registry, Resolved,
};

use crate::state::AppState;

/// This app's desktop file / executable name, to recognise its own handler.
const OWN: &str = "tmc";

fn availability() -> ProviderAvailability {
    // TODO(site): fetch from the site once an endpoint exists; see
    // docs/external-mods.md. Until then every keyed provider is off.
    ProviderAvailability::disabled()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderState {
    pub provider: ProviderId,
    pub label: String,
    pub state: CredentialState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedExternal {
    pub reference: ExternalRef,
    pub provider: ProviderState,
}

/// Recognise a pasted link. Pure; no network.
#[tauri::command]
pub fn external_parse(text: String) -> AppResult<ParsedExternal> {
    let reference = parse_input(&text).map_err(|e| AppError::invalid(e.to_string()))?;
    let registry = Registry::standard();
    let avail = availability();
    let state = registry
        .get(reference.provider)
        .map(|p| p.credential_state(&avail))
        .unwrap_or(CredentialState::NotConfigured {
            reason: "no provider".into(),
        });
    Ok(ParsedExternal {
        provider: ProviderState {
            provider: reference.provider,
            label: reference.provider.label().to_string(),
            state,
        },
        reference,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedExternal {
    pub resolved: Resolved,
    pub candidates: Vec<Candidate>,
}

/// Look a link up at its provider and suggest which game or sandbox fits.
#[tauri::command]
pub async fn external_resolve(
    state: State<'_, AppState>,
    text: String,
    loader: Option<String>,
    game_version: Option<String>,
) -> AppResult<ResolvedExternal> {
    let r = parse_input(&text).map_err(|e| AppError::invalid(e.to_string()))?;
    let registry = Registry::standard();
    let provider = registry
        .get(r.provider)
        .ok_or_else(|| AppError::invalid("no provider handles that link"))?;
    if let CredentialState::NotConfigured { reason } = provider.credential_state(&availability()) {
        return Err(AppError::invalid(reason));
    }
    let resolved = provider
        .resolve(
            &HttpFetch::new(),
            &r,
            &ResolveCtx {
                loader,
                game_version,
            },
        )
        .await?;
    let candidates = candidates_for(&state, &resolved, None)?;
    Ok(ResolvedExternal {
        resolved,
        candidates,
    })
}

fn candidates_for(
    state: &AppState,
    resolved: &Resolved,
    meta: Option<&meta::ExternalMeta>,
) -> AppResult<Vec<Candidate>> {
    let sandboxes = state.library.sandbox_list(None)?;
    let mut games: Vec<GameRef> = Vec::new();
    for s in &sandboxes {
        if !games.iter().any(|g| g.app_id == s.app_id) {
            games.push(GameRef {
                app_id: s.app_id,
                slug: s.app_slug.clone(),
                name: s.app_name.clone().unwrap_or_default(),
            });
        }
    }
    let refs: Vec<SandboxRef> = sandboxes
        .iter()
        .map(|s| SandboxRef {
            id: s.id,
            app_id: s.app_id,
            name: s.name.clone(),
            loader: s.loader.clone(),
            game_version: s.game_version.clone(),
            is_default: s.is_default,
        })
        .collect();
    Ok(matching::candidates(&resolved.game, meta, &games, &refs))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRequest {
    pub text: String,
    pub app_id: Option<i64>,
    pub sandbox_id: Option<i64>,
    pub loader: Option<String>,
    pub game_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub outcome: AddOutcome,
    /// The sandbox the mod was added to, when one was named.
    pub sandbox_id: Option<i64>,
}

/// Install what the user pasted or confirmed. Downloads, verifies, unpacks.
#[tauri::command]
pub async fn external_install(
    state: State<'_, AppState>,
    request: InstallRequest,
) -> AppResult<InstallReport> {
    let sandbox = match request.sandbox_id {
        Some(id) => Some(
            state
                .library
                .sandbox_get(id)?
                .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?,
        ),
        None => None,
    };
    let app_id = sandbox.as_ref().map(|s| s.app_id).or(request.app_id);
    let slug = sandbox
        .as_ref()
        .and_then(|s| s.app_slug.clone())
        .or_else(|| app_id.and_then(|id| state.app_slug_for(id)));
    let plugins = state.app_plugins();
    let targets = slug
        .as_deref()
        .map(|s| plugins.mod_targets(s))
        .unwrap_or_default();
    let extensions = slug
        .as_deref()
        .map(|s| plugins.mod_extensions(s))
        .unwrap_or_default();
    let store_root = state.paths.local_mods_dir();
    let env = InstallEnv {
        db: &state.library,
        store_root: &store_root,
        mod_targets: &targets,
        mod_extensions: &extensions,
    };
    let outcome = install::add(
        &request.text,
        &HttpFetch::new(),
        &Registry::standard(),
        &availability(),
        &ResolveCtx {
            loader: request.loader,
            game_version: request.game_version,
        },
        &state.downloads,
        &store_root.join(".downloads"),
        state.settings_snapshot().external_max_download_mb,
        &env,
        &Target {
            app_id,
            app_slug: slug,
        },
    )
    .await?;
    let mut added_to = None;
    if let (AddOutcome::Installed(i), Some(sb)) = (&outcome, &sandbox) {
        if let Some(local) = state.library.local_get(i.local_id)? {
            state.library.sandbox_add_local(sb.id, &local)?;
            added_to = Some(sb.id);
        }
    }
    Ok(InstallReport {
        outcome,
        sandbox_id: added_to,
    })
}

/// Remove a mod that was added from a link. Returns sandboxes needing a redeploy.
#[tauri::command]
pub fn external_remove(state: State<'_, AppState>, local_id: i64) -> AppResult<Vec<i64>> {
    install::remove(&state.library, &state.paths.local_mods_dir(), local_id)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectReport {
    pub inspection: Inspection,
    pub candidates: Vec<Candidate>,
}

/// Read the metadata of a dropped file or folder, named by its vault token.
#[tauri::command]
pub fn external_inspect_path(
    state: State<'_, AppState>,
    token: String,
    app_id: Option<i64>,
) -> AppResult<InspectReport> {
    let path = state
        .vault
        .resolve(&token)
        .ok_or_else(|| AppError::invalid("That file is no longer available — drop it again."))?;
    let slug = app_id.and_then(|id| state.app_slug_for(id));
    let extensions = slug
        .as_deref()
        .map(|s| state.app_plugins().mod_extensions(s))
        .unwrap_or_default();
    let inspection = meta::inspect(&Source::open(&path)?, &extensions)?;
    let found = match &inspection {
        Inspection::Found { meta, .. } => Some(meta.clone()),
        Inspection::NeedsSelection { .. } => None,
    };
    let stub = Resolved {
        provider: ProviderId::Direct,
        project_id: String::new(),
        version_id: String::new(),
        title: String::new(),
        version: String::new(),
        author: None,
        game: Default::default(),
        files: vec![],
        deps: vec![],
        page_url: None,
        manual_only: None,
        licence: None,
    };
    let candidates = candidates_for(&state, &stub, found.as_ref())?;
    Ok(InspectReport {
        inspection,
        candidates,
    })
}

/// Route a link the OS handed over. Never installs. For `lib.rs`.
pub fn route_link(url: &str) -> Option<Pending> {
    pending::route(url)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemeReport {
    pub scheme: String,
    pub status: SchemeStatus,
}

/// Who owns each external link scheme. Read-only.
#[tauri::command]
pub fn external_scheme_status() -> AppResult<Vec<SchemeReport>> {
    let os = Os::current();
    scheme::MANAGED
        .iter()
        .map(|s| {
            Ok(SchemeReport {
                scheme: (*s).to_string(),
                status: scheme::status(&SystemRunner, os, s, OWN)?,
            })
        })
        .collect()
}

/// Claim (`enable`) or release a scheme. Never replaces another app's handler
/// unless `override_existing` is true.
#[tauri::command]
pub fn external_scheme_set(
    scheme: String,
    enable: bool,
    override_existing: Option<bool>,
) -> AppResult<SchemeReport> {
    let os = Os::current();
    let status = if enable {
        let exe = std::env::current_exe()
            .map_err(|e| AppError::internal(e.to_string()))?
            .to_string_lossy()
            .into_owned();
        scheme::set(
            &SystemRunner,
            os,
            &scheme,
            OWN,
            &exe,
            "tmc.desktop",
            override_existing.unwrap_or(false),
        )?
    } else {
        scheme::unset(&SystemRunner, os, &scheme, OWN)?
    };
    Ok(SchemeReport { scheme, status })
}
