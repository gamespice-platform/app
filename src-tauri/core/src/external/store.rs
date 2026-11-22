//! `external_mod`: where each external local mod came from.
//!
//! A side table keyed by the `local_mod` id rather than columns on it: the
//! local-mod schema belongs to the library, and `SourceRef` there carries an
//! integer item id that no string-keyed provider can fill.

use serde::{Deserialize, Serialize};

use super::{ExternalSource, ProviderId};
use crate::error::{AppError, AppResult};
use crate::library::LibraryDb;

const DDL: &str = "CREATE TABLE IF NOT EXISTS external_mod (
    local_id    INTEGER PRIMARY KEY,
    provider    TEXT NOT NULL,
    url         TEXT NOT NULL,
    project_id  TEXT NOT NULL,
    version_id  TEXT NOT NULL,
    version     TEXT NOT NULL,
    sha512      TEXT,
    verified    TEXT NOT NULL DEFAULT 'none'
)";

/// How the download was checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalRow {
    pub local_id: i64,
    pub source: ExternalSource,
    /// `sha512`, `sha256`, `size` or `none`.
    pub verified: String,
}

pub fn put(db: &LibraryDb, row: &ExternalRow) -> AppResult<()> {
    db.with(|c| {
        c.execute(DDL, [])?;
        c.execute(
            "INSERT OR REPLACE INTO external_mod (local_id, provider, url, project_id, version_id, version, verified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                row.local_id,
                row.source.provider.as_str(),
                row.source.url,
                row.source.project_id,
                row.source.version_id,
                row.source.version,
                row.verified
            ],
        )?;
        Ok(())
    })
}

pub fn get(db: &LibraryDb, local_id: i64) -> AppResult<Option<ExternalRow>> {
    db.with(|c| {
        c.execute(DDL, [])?;
        let mut st = c.prepare(
            "SELECT provider, url, project_id, version_id, version, verified FROM external_mod WHERE local_id = ?1",
        )?;
        let mut rows = st.query([local_id])?;
        let Some(r) = rows.next()? else { return Ok(None) };
        let provider: String = r.get(0)?;
        Ok(Some((provider, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
    })
    .map(|o: Option<(String, String, String, String, String, String)>| {
        o.and_then(|(p, url, project_id, version_id, version, verified)| {
            Some(ExternalRow {
                local_id,
                source: ExternalSource { provider: ProviderId::parse(&p)?, url, project_id, version_id, version },
                verified,
            })
        })
    })
}

pub fn list(db: &LibraryDb) -> AppResult<Vec<ExternalRow>> {
    let ids: Vec<i64> = db.with(|c| {
        c.execute(DDL, [])?;
        let mut st = c.prepare("SELECT local_id FROM external_mod ORDER BY local_id")?;
        let v = st
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<i64>, _>>()?;
        Ok(v)
    })?;
    ids.into_iter()
        .filter_map(|i| get(db, i).transpose())
        .collect()
}

pub fn delete(db: &LibraryDb, local_id: i64) -> AppResult<()> {
    db.with(|c| {
        c.execute(DDL, [])?;
        c.execute("DELETE FROM external_mod WHERE local_id = ?1", [local_id])?;
        Ok(())
    })
    .map_err(|e| AppError::internal(format!("external_mod delete: {}", e.detail())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::ProviderId;

    #[test]
    fn put_get_list_delete_round_trip() {
        let db = LibraryDb::open_memory().unwrap();
        assert!(get(&db, 1).unwrap().is_none());
        let row = ExternalRow {
            local_id: 1,
            source: ExternalSource {
                provider: ProviderId::Github,
                url: "https://github.com/a/b".into(),
                project_id: "a/b".into(),
                version_id: "v1".into(),
                version: "1.0".into(),
            },
            verified: "sha256".into(),
        };
        put(&db, &row).unwrap();
        assert_eq!(get(&db, 1).unwrap().unwrap(), row);
        assert_eq!(list(&db).unwrap().len(), 1);
        delete(&db, 1).unwrap();
        assert!(get(&db, 1).unwrap().is_none());
    }
}
