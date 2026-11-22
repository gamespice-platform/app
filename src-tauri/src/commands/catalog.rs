//! The version and loader pickers' lists (docs/loaders-and-versions.md,
//! "Lists: remote, cached, compiled in"), read through the catalog client:
//! the cache within `catalogTtlMinutes`, then the website, then the copy
//! compiled into this build. Every answer says which of the three it is.

use tauri::State;
use tmc_core::catalog::types::{CatalogView, Channel, LoaderVersionsView};
use tmc_core::error::AppResult;

use crate::state::AppState;

/// `versionListCount` unless the caller asks for another limit; `0` is all.
fn limit_of(state: &AppState, limit: Option<u32>) -> u32 {
    limit.unwrap_or_else(|| state.settings.get().version_list_count)
}

/// A game's loaders and its versions in `channels` (STABLE when empty).
/// `refresh` skips the cache's freshness window and asks the website.
#[tauri::command]
pub async fn catalog_get(
    state: State<'_, AppState>,
    app_id: i64,
    app_slug: String,
    channels: Option<Vec<Channel>>,
    limit: Option<u32>,
    refresh: Option<bool>,
) -> AppResult<CatalogView> {
    let client = state.catalog_client();
    tmc_core::catalog::catalog(
        &client,
        app_id,
        &app_slug,
        &channels.unwrap_or_default(),
        limit_of(&state, limit),
        refresh.unwrap_or(false),
    )
    .await
}

/// One loader's versions, only those with a build for `game_version` when
/// one is given.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn catalog_loader_versions(
    state: State<'_, AppState>,
    app_id: i64,
    app_slug: String,
    loader: String,
    game_version: Option<String>,
    channels: Option<Vec<Channel>>,
    limit: Option<u32>,
    refresh: Option<bool>,
) -> AppResult<LoaderVersionsView> {
    let client = state.catalog_client();
    tmc_core::catalog::loader_versions(
        &client,
        app_id,
        &app_slug,
        &loader,
        game_version.as_deref(),
        &channels.unwrap_or_default(),
        limit_of(&state, limit),
        refresh.unwrap_or(false),
    )
    .await
}
