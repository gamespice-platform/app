//! The game catalog the website serves (docs/loaders-and-versions.md, "Lists:
//! remote, cached, compiled in"): the version and loader pickers' lists and
//! the install plans deploy resolves, read through one policy
//! ([`client::CatalogClient`]) of cache, then network, then the snapshot
//! compiled into this build.
//!
//! The wire shapes are in [`types`], the cache table (library database
//! migration 11) in [`cache`], the compiled fallback in [`fallback`], and the
//! served plan's conversion to a [`PlanSource`] in [`wire_plan`].

pub mod cache;
pub mod client;
pub mod fallback;
pub mod types;
pub mod wire_plan;

#[cfg(test)]
mod tests;

pub use client::{CatalogClient, Fetched, Got, Transport};

use crate::error::AppResult;
use crate::layers::plan::{PlanRequest, PlanSource};

use types::{
    CatalogView, CatalogWire, Channel, InstallPlanWire, LoaderVersionList, LoaderVersionsView,
    Provenance, Source,
};

const COMPILED: Provenance = Provenance {
    source: Source::Compiled,
    stale: false,
    fetched_at: None,
};

/// `catalog_get`: the game's loaders and its versions in `channels`, at most
/// `limit` of them (`versionListCount`; `0` is all the server will send).
pub async fn catalog<T: Transport>(
    client: &CatalogClient<'_, T>,
    app_id: i64,
    slug: &str,
    channels: &[Channel],
    limit: u32,
    force: bool,
) -> AppResult<CatalogView> {
    let path = client::catalog_path(app_id, channels, limit);
    if let Some(Got { body, provenance }) = client.get::<CatalogWire>(&path, force).await? {
        return Ok(CatalogView {
            app: body.app,
            loaders: body.loaders,
            versions: body.versions,
            generated_at: Some(body.generated_at),
            provenance,
        });
    }
    let snap = fallback::catalog(
        &fallback::snapshot(PlanSource::compiled(), slug)?,
        channels,
        limit,
    );
    Ok(CatalogView {
        app: snap.app,
        loaders: snap.loaders,
        versions: snap.versions,
        generated_at: snap.generated_at,
        provenance: COMPILED,
    })
}

/// `catalog_loader_versions`: one loader's versions, narrowed to those with a
/// build for `game_version` when one is given.
#[allow(clippy::too_many_arguments)]
pub async fn loader_versions<T: Transport>(
    client: &CatalogClient<'_, T>,
    app_id: i64,
    slug: &str,
    loader: &str,
    game_version: Option<&str>,
    channels: &[Channel],
    limit: u32,
    force: bool,
) -> AppResult<LoaderVersionsView> {
    let path = client::loader_versions_path(app_id, loader, game_version, channels, limit)?;
    if let Some(Got { body, provenance }) = client.get::<LoaderVersionList>(&path, force).await? {
        return Ok(LoaderVersionsView {
            loader: body.loader,
            game_version: body.game_version,
            items: body.items,
            next_cursor: body.next_cursor,
            provenance,
        });
    }
    let snap = fallback::snapshot(PlanSource::compiled(), slug)?;
    Ok(LoaderVersionsView {
        loader: loader.to_string(),
        game_version: game_version.map(str::to_string),
        items: fallback::loader_versions(&snap, loader, game_version, channels, limit),
        next_cursor: None,
        provenance: COMPILED,
    })
}

/// The plan source deploy resolves `req` against: the served plan (fresh or
/// cached), or `None` for the compiled-in source. A withdrawn build is an
/// error, never a silent fallback to an older plan.
pub async fn plan_source<T: Transport>(
    client: &CatalogClient<'_, T>,
    app_id: i64,
    req: &PlanRequest,
) -> AppResult<Option<(PlanSource, Provenance)>> {
    let path = client::install_plan_path(
        app_id,
        &req.game_version,
        req.loader.as_deref(),
        req.loader_version.as_deref(),
        &req.platform,
        &req.side,
    );
    let Some(Got { body, provenance }) = client.get::<InstallPlanWire>(&path, false).await? else {
        return Ok(None);
    };
    let source = wire_plan::plan_source(&body, req, &PlanSource::compiled().keys)?;
    Ok(Some((source, provenance)))
}
