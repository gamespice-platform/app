//! Cache, then network, then the compiled snapshot.
//!
//! One rule for every catalog read, in this order:
//!
//! 1. A cached copy younger than `catalogTtlMinutes` is served as it is
//!    (`0` means always revalidate).
//! 2. Otherwise the request goes out with the cached ETag as `If-None-Match`;
//!    a 304 refreshes the copy's age, a 200 replaces it.
//! 3. A network failure (or a server error, or a body that is not the agreed
//!    shape) serves the cached copy flagged `stale`.
//! 4. With no cached copy the caller falls back to the compiled snapshot.
//!
//! Two refusals are not failures to paper over: `catalog_withdrawn` (410) is an
//! error carrying the admin's note, and `catalog_not_found`,
//! `catalog_unmanaged` (404) and `catalog_platform` (422) forget the cached
//! copy, since the server has said it no longer stands, and fall back to the
//! snapshot.

use std::future::Future;

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::library::db::LibraryDb;

use super::cache::{self, Cached};
use super::types::{Channel, Provenance, Source};

/// What one conditional GET came back with.
#[derive(Debug, Clone, PartialEq)]
pub enum Fetched {
    /// 304: the cached copy stands.
    NotModified,
    /// 200: the envelope's `data` and the response's ETag.
    Body { etag: Option<String>, data: Value },
    /// The API's own error envelope.
    Refused {
        status: u16,
        code: String,
        message: String,
        note: Option<String>,
    },
}

/// The network side, so the policy above is tested without one.
/// [`crate::api::ApiClient`] is the real one. An `Err` is a failure to reach
/// the server at all.
pub trait Transport: Send + Sync {
    fn get(
        &self,
        path: &str,
        etag: Option<&str>,
    ) -> impl Future<Output = AppResult<Fetched>> + Send;
}

impl Transport for crate::api::ApiClient {
    fn get(
        &self,
        path: &str,
        etag: Option<&str>,
    ) -> impl Future<Output = AppResult<Fetched>> + Send {
        self.get_conditional(path, etag)
    }
}

/// A body read through the policy, and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Got<T> {
    pub body: T,
    pub provenance: Provenance,
}

/// The catalog reads of one app process.
pub struct CatalogClient<'a, T: Transport> {
    pub db: &'a LibraryDb,
    pub transport: &'a T,
    /// `catalogTtlMinutes`; `0` revalidates every read.
    pub ttl_minutes: u32,
    /// Unix milliseconds. A field so tests can move the clock.
    pub now: fn() -> i64,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

impl<'a, T: Transport> CatalogClient<'a, T> {
    pub fn new(db: &'a LibraryDb, transport: &'a T, ttl_minutes: u32) -> Self {
        Self {
            db,
            transport,
            ttl_minutes,
            now: now_ms,
        }
    }

    /// `path` through the policy. `Ok(None)` means "use the compiled
    /// snapshot": nothing cached and nothing fetched, or the server said the
    /// request has no answer. `force` skips the TTL (`catalog_refresh`).
    pub async fn get<B: DeserializeOwned>(
        &self,
        path: &str,
        force: bool,
    ) -> AppResult<Option<Got<B>>> {
        let now = (self.now)();
        let cached = match cache::read(self.db, path)? {
            // A row that no longer parses (an older app wrote it, or the shape
            // moved) is as good as none.
            Some(c) => match serde_json::from_str::<B>(&c.body) {
                Ok(body) => Some((c, body)),
                Err(_) => {
                    cache::delete(self.db, path)?;
                    None
                }
            },
            None => None,
        };

        if let Some((c, _)) = &cached {
            let ttl_ms = i64::from(self.ttl_minutes) * 60_000;
            if !force && ttl_ms > 0 && now.saturating_sub(c.fetched_at) < ttl_ms {
                let (c, body) = cached.expect("checked above");
                return Ok(Some(served(body, Source::Cache, false, c.fetched_at)));
            }
        }

        let etag = cached.as_ref().and_then(|(c, _)| c.etag.as_deref());
        let stale = |cached: Option<(Cached, B)>| {
            cached.map(|(c, body)| served(body, Source::Cache, true, c.fetched_at))
        };

        match self.transport.get(path, etag).await {
            Ok(Fetched::NotModified) => match cached {
                Some((c, body)) => {
                    cache::touch(self.db, path, now)?;
                    let _ = c;
                    Ok(Some(served(body, Source::Cache, false, now)))
                }
                // A 304 to a request that sent no ETag is the server's bug;
                // there is nothing to confirm.
                None => Ok(None),
            },
            Ok(Fetched::Body { etag, data }) => {
                let text = data.to_string();
                match serde_json::from_value::<B>(data) {
                    Ok(body) => {
                        cache::write(
                            self.db,
                            path,
                            &Cached {
                                etag,
                                body: text,
                                fetched_at: now,
                            },
                        )?;
                        Ok(Some(served(body, Source::Remote, false, now)))
                    }
                    Err(e) => {
                        tracing::warn!(
                            "catalog {path}: the response is not the agreed shape ({e})"
                        );
                        Ok(stale(cached))
                    }
                }
            }
            Ok(Fetched::Refused {
                status: 410,
                message,
                note,
                ..
            }) => Err(withdrawn(&message, note.as_deref())),
            Ok(Fetched::Refused {
                status: 404 | 422,
                code,
                ..
            }) => {
                tracing::info!("catalog {path}: {code}; forgetting the cached copy");
                cache::delete(self.db, path)?;
                Ok(None)
            }
            Ok(Fetched::Refused { status, code, .. }) => {
                tracing::warn!("catalog {path}: {status} {code}; serving what is cached");
                Ok(stale(cached))
            }
            Err(e) => {
                tracing::info!("catalog {path}: {e}; serving what is cached");
                Ok(stale(cached))
            }
        }
    }
}

fn served<B>(body: B, source: Source, stale: bool, fetched_at: i64) -> Got<B> {
    Got {
        body,
        provenance: Provenance {
            source,
            stale,
            fetched_at: Some(fetched_at),
        },
    }
}

/// `catalog_withdrawn`, with the admin's note when there is one.
pub fn withdrawn(message: &str, note: Option<&str>) -> AppError {
    match note.filter(|n| !n.trim().is_empty()) {
        Some(note) => AppError::Invalid(format!("{message} {note}")),
        None => AppError::Invalid(message.to_string()),
    }
}

// ------------------------------------------------------------------- paths

/// `?channels=` as the server reads it, or nothing for its default (STABLE).
fn channels_param(channels: &[Channel]) -> Option<String> {
    if channels.is_empty() || channels == [Channel::Stable] {
        return None;
    }
    let mut seen: Vec<&str> = Vec::new();
    for c in Channel::ALL {
        if channels.contains(&c) {
            seen.push(c.as_str());
        }
    }
    Some(seen.join(","))
}

fn query(pairs: &[(&str, Option<String>)]) -> String {
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        if let Some(v) = v {
            q.append_pair(k, v);
        }
    }
    let q = q.finish();
    if q.is_empty() {
        q
    } else {
        format!("?{q}")
    }
}

/// A loader key as a path segment: refused rather than escaped, since a key
/// is a lowercase slug and anything else is a caller's mistake.
fn segment(key: &str) -> AppResult<&str> {
    let ok = !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        && key != "."
        && key != "..";
    if ok {
        Ok(key)
    } else {
        Err(AppError::invalid(format!("{key:?} is not a loader key.")))
    }
}

/// `limit` as sent: `0` asks the server for its maximum.
pub fn catalog_path(app_id: i64, channels: &[Channel], limit: u32) -> String {
    format!(
        "/apps/{app_id}/catalog{}",
        query(&[
            ("limit", Some(limit.to_string())),
            ("channels", channels_param(channels)),
        ])
    )
}

pub fn loader_versions_path(
    app_id: i64,
    loader: &str,
    game_version: Option<&str>,
    channels: &[Channel],
    limit: u32,
) -> AppResult<String> {
    Ok(format!(
        "/apps/{app_id}/loaders/{}/versions{}",
        segment(loader)?,
        query(&[
            ("gameVersion", game_version.map(str::to_string)),
            ("limit", Some(limit.to_string())),
            ("channels", channels_param(channels)),
        ])
    ))
}

pub fn install_plan_path(
    app_id: i64,
    game_version: &str,
    loader: Option<&str>,
    loader_version: Option<&str>,
    platform: &str,
    side: &str,
) -> String {
    format!(
        "/apps/{app_id}/install-plan{}",
        query(&[
            ("gameVersion", Some(game_version.to_string())),
            ("loader", loader.map(str::to_string)),
            ("loaderVersion", loader_version.map(str::to_string)),
            ("platform", Some(platform.to_string())),
            ("side", Some(side.to_string())),
        ])
    )
}
