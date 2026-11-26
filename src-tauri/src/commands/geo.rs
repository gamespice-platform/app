//! The opt-in latency estimate: database status, source, download, delete, and
//! the estimate itself. The logic is `tmc_core::net::geo_store`; this layer only
//! reads the `estimate_latency` setting and the token, so the webview never
//! holds either the token or a download URL it can aim a token at.
//!
//! With the setting off, `geo_download` and `geo_estimate` refuse, and
//! `geo_status` answers without touching the network.

use serde::Deserialize;
use tauri::State;

use tmc_core::audit;
use tmc_core::error::{AppError, AppResult};
use tmc_core::net::geo_store::{GeoConfig, GeoStatus, TOKEN_NAME};

use crate::state::AppState;

fn has_token(state: &AppState) -> bool {
    state
        .secure
        .load_named(TOKEN_NAME)
        .is_some_and(|t| !t.trim().is_empty())
}

#[tauri::command]
pub fn geo_status(state: State<'_, AppState>) -> GeoStatus {
    state.geo.status(has_token(&state))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeoConfigure {
    /// URL template with `{token}`; absent keeps the current one.
    pub source: Option<String>,
    pub self_ip_url: Option<String>,
    /// A new token; `""` clears it; absent keeps the current one.
    pub token: Option<String>,
    /// Put the source and own-IP URL back to the defaults.
    #[serde(default)]
    pub reset: bool,
}

#[tauri::command]
pub fn geo_configure(state: State<'_, AppState>, input: GeoConfigure) -> AppResult<GeoStatus> {
    let mut cfg = if input.reset {
        GeoConfig::default()
    } else {
        state.geo.config()
    };

    if let Some(s) = input.source {
        cfg.source = s.trim().to_string();
    }

    if let Some(s) = input.self_ip_url {
        cfg.self_ip_url = s.trim().to_string();
    }

    state.geo.set_config(&cfg)?;

    if let Some(t) = input.token {
        let t = t.trim();

        if t.len() > 256 {
            return Err(AppError::invalid(
                "That token is far too long to be a download token.",
            ));
        }

        state.secure.save_named(TOKEN_NAME, t)?;
    }

    audit!(
        state.audit,
        Info,
        App,
        "geo.configure",
        "Latency estimate source changed" // never the token or the URL
    );

    Ok(state.geo.status(has_token(&state)))
}

/// Download (or refresh) the database. Resolves when finished; progress is
/// polled through `geo_status`.
#[tauri::command]
pub async fn geo_download(state: State<'_, AppState>) -> AppResult<GeoStatus> {
    let enabled = state.settings.get().estimate_latency;
    let token = state.secure.load_named(TOKEN_NAME);
    let res = state.geo.download(enabled, token, &state.geo_http).await;

    match &res {
        Ok(()) => audit!(
            state.audit,
            Info,
            App,
            "geo.download.ok",
            "Geolocation database updated"
        ),
        Err(e) => audit!(state.audit, Warn, App, "geo.download.fail", e.to_string()),
    }

    res?;
    Ok(state.geo.status(has_token(&state)))
}

#[tauri::command]
pub fn geo_delete(state: State<'_, AppState>) -> AppResult<GeoStatus> {
    state.geo.delete()?;
    audit!(
        state.audit,
        Info,
        App,
        "geo.delete",
        "Geolocation database deleted"
    );
    Ok(state.geo.status(has_token(&state)))
}

/// Estimated ms for each host, in order; `null` where unknown. Refreshes an
/// old database in the background first when it is due.
#[tauri::command]
pub async fn geo_estimate(
    state: State<'_, AppState>,
    hosts: Vec<String>,
) -> AppResult<Vec<Option<u32>>> {
    let enabled = state.settings.get().estimate_latency;

    if hosts.len() > 256 {
        return Err(AppError::invalid("Too many hosts in one request."));
    }

    // Lazy periodic refresh: an old file is replaced after being used, never
    // blocking the answer. A failure is recorded in the status, not raised here.
    if enabled
        && state
            .geo
            .refresh_due(tmc_core::net::geo_store::REFRESH_DAYS)
        && has_token(&state)
    {
        let token = state.secure.load_named(TOKEN_NAME);
        let _ = state.geo.download(enabled, token, &state.geo_http).await;
    }

    state.geo.estimate(enabled, &hosts, &state.geo_http).await
}
