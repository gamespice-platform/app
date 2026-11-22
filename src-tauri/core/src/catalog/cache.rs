//! The `catalog_cache` table (library database migration 11): one row per
//! request path plus query, holding the last good body and its ETag.

use rusqlite::{params, OptionalExtension};

use crate::error::AppResult;
use crate::library::db::LibraryDb;

/// One cached response.
#[derive(Debug, Clone, PartialEq)]
pub struct Cached {
    pub etag: Option<String>,
    pub body: String,
    /// Unix milliseconds of the fetch, or of the last 304 that confirmed it.
    pub fetched_at: i64,
}

pub fn read(db: &LibraryDb, key: &str) -> AppResult<Option<Cached>> {
    db.with(|conn| {
        conn.query_row(
            "SELECT etag, body, fetched_at FROM catalog_cache WHERE key = ?1",
            params![key],
            |r| {
                Ok(Cached {
                    etag: r.get(0)?,
                    body: r.get(1)?,
                    fetched_at: r.get(2)?,
                })
            },
        )
        .optional()
    })
}

pub fn write(db: &LibraryDb, key: &str, entry: &Cached) -> AppResult<()> {
    db.with(|conn| {
        conn.execute(
            "INSERT INTO catalog_cache (key, etag, body, fetched_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET etag = excluded.etag, body = excluded.body,
                 fetched_at = excluded.fetched_at",
            params![key, entry.etag, entry.body, entry.fetched_at],
        )?;
        Ok(())
    })
}

/// A 304: the body is still right, so only its age changes.
pub fn touch(db: &LibraryDb, key: &str, fetched_at: i64) -> AppResult<()> {
    db.with(|conn| {
        conn.execute(
            "UPDATE catalog_cache SET fetched_at = ?2 WHERE key = ?1",
            params![key, fetched_at],
        )?;
        Ok(())
    })
}

pub fn delete(db: &LibraryDb, key: &str) -> AppResult<()> {
    db.with(|conn| {
        conn.execute("DELETE FROM catalog_cache WHERE key = ?1", params![key])?;
        Ok(())
    })
}
