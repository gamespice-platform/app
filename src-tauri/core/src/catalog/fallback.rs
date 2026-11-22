//! The compiled-in snapshot (`manifests/<app>/catalog.snapshot.json`), read as
//! the lists the pickers show when there is no cached copy and no network.

use crate::error::{AppError, AppResult};
use crate::layers::plan::PlanSource;

use super::types::{Channel, LoaderVersion, Page, Snapshot};

/// The snapshot for `slug`, or an error naming the game: with nothing cached,
/// no network and nothing compiled in, there is no list to show.
pub fn snapshot(source: &PlanSource, slug: &str) -> AppResult<Snapshot> {
    let raw = source.catalogs.get(slug).ok_or_else(|| {
        AppError::Network(format!(
            "The {slug} version list could not be fetched, and this app has no copy of it. Connect to the internet and try again."
        ))
    })?;
    serde_json::from_value(raw.clone())
        .map_err(|e| AppError::internal(format!("the compiled {slug} catalog is malformed: {e}")))
}

/// `channels` empty means the server's default, STABLE.
pub fn wanted(channel: Channel, channels: &[Channel]) -> bool {
    if channels.is_empty() {
        channel == Channel::Stable
    } else {
        channels.contains(&channel)
    }
}

/// `limit` `0` is every row.
pub fn truncate<T>(mut items: Vec<T>, limit: u32) -> Vec<T> {
    if limit > 0 {
        items.truncate(limit as usize);
    }
    items
}

/// The snapshot's catalog, filtered the way the server would filter it.
pub fn catalog(snap: &Snapshot, channels: &[Channel], limit: u32) -> Snapshot {
    let items = snap
        .versions
        .items
        .iter()
        .filter(|v| wanted(v.channel, channels))
        .cloned()
        .collect();
    Snapshot {
        versions: Page {
            items: truncate(items, limit),
            next_cursor: None,
        },
        ..snap.clone()
    }
}

/// One loader's versions from the snapshot; with `game_version`, only those
/// with a published build for it.
pub fn loader_versions(
    snap: &Snapshot,
    loader: &str,
    game_version: Option<&str>,
    channels: &[Channel],
    limit: u32,
) -> Vec<LoaderVersion> {
    let rows = snap
        .loader_versions
        .get(loader)
        .cloned()
        .unwrap_or_default();
    let items = rows
        .into_iter()
        .filter(|v| wanted(v.channel, channels))
        .filter(|v| {
            game_version.is_none_or(|gv| {
                snap.builds.iter().any(|b| {
                    b.loader == loader && b.loader_version == v.version && b.game_version == gv
                })
            })
        })
        .collect();
    truncate(items, limit)
}
