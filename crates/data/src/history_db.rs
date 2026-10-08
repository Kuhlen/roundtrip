//! Sent requests in SQLite at `<config dir>/roundtrip/history.db`, ApiArk's columns.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, SecondsFormat, Utc};
use domain::AppError;
use domain::history::{HistoryEntry, HistoryStore};
use domain::http::{Method, Request};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, params};

use crate::collection_dir::{request_from_yaml, request_to_yaml};
use crate::storage;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    method TEXT NOT NULL,
    url TEXT NOT NULL,
    status INTEGER,
    status_text TEXT,
    time_ms INTEGER,
    size_bytes INTEGER,
    timestamp TEXT NOT NULL,
    collection_path TEXT,
    request_name TEXT,
    request_yaml TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_history_timestamp ON history(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_history_url ON history(url);
CREATE INDEX IF NOT EXISTS idx_history_method ON history(method);
";

const COLUMNS: &str = "id, method, url, status, status_text, time_ms, size_bytes, timestamp, collection_path, request_name";

pub struct HistoryDb {
    conn: Mutex<Connection>,
}

enum OpenError {
    Corrupt,
    Failed(AppError),
}

impl HistoryDb {
    /// A corrupt file is moved to `<name>.corrupt.<timestamp>` and a fresh one created.
    pub fn open(path: &Path) -> Result<Self, AppError> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| storage(dir, e))?;
        }
        let conn = match connect(path) {
            Ok(conn) => conn,
            Err(OpenError::Corrupt) => {
                set_aside(path)?;
                connect(path).map_err(OpenError::into_app)?
            }
            Err(OpenError::Failed(e)) => return Err(e),
        };
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn in_config_dir() -> Result<Self, AppError> {
        let base = dirs::config_dir().unwrap_or_else(std::env::temp_dir);
        Self::open(&base.join("roundtrip").join("history.db"))
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // a panicked holder left no half row: SQLite statements are atomic
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn query(
        &self,
        sql: &str,
        args: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<HistoryEntry>, AppError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(sql).map_err(db)?;
        let rows = stmt.query_map(args, entry).map_err(db)?;
        rows.collect::<Result<_, _>>().map_err(db)
    }
}

impl OpenError {
    fn into_app(self) -> AppError {
        match self {
            OpenError::Corrupt => AppError::History("database is corrupt".into()),
            OpenError::Failed(e) => e,
        }
    }
}

// ponytail: no full integrity_check on start (reads the whole file); deep damage shows as a query error
fn connect(path: &Path) -> Result<Connection, OpenError> {
    let conn = Connection::open(path).map_err(open_error)?;
    // another Roundtrip writing is busy, not corrupt
    conn.busy_timeout(Duration::from_secs(2))
        .map_err(open_error)?;
    // first read: a non-SQLite file fails here with NotADatabase
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(open_error)?;
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))
        .map_err(open_error)?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(open_error)?;
    conn.execute_batch(SCHEMA).map_err(open_error)?;
    Ok(conn)
}

fn open_error(e: rusqlite::Error) -> OpenError {
    match e.sqlite_error_code() {
        Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt) => OpenError::Corrupt,
        _ => OpenError::Failed(db(e)),
    }
}

fn set_aside(path: &Path) -> Result<(), AppError> {
    let stamp = Utc::now().format("%Y%m%d%H%M%S");
    fs::rename(path, sibling(path, &format!(".corrupt.{stamp}"))).map_err(|e| storage(path, e))?;
    // a stale WAL would be replayed into the fresh file
    for side in ["-wal", "-shm"] {
        let _ = fs::remove_file(sibling(path, side));
    }
    Ok(())
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

fn db(e: rusqlite::Error) -> AppError {
    AppError::History(e.to_string())
}

fn int(v: Option<u64>) -> Option<i64> {
    v.map(|v| i64::try_from(v).unwrap_or(i64::MAX))
}

fn uint(v: Option<i64>) -> Option<u64> {
    v.map(|v| u64::try_from(v).unwrap_or(0))
}

/// LIKE wildcards in the query are text, not patterns.
fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

fn entry(row: &Row) -> rusqlite::Result<HistoryEntry> {
    let method: String = row.get(1)?;
    let at: String = row.get(7)?;
    Ok(HistoryEntry {
        id: row.get(0)?,
        method: Method::parse(&method).unwrap_or_default(),
        url: row.get(2)?,
        status: row.get(3)?,
        status_text: row.get(4)?,
        time_ms: uint(row.get(5)?),
        size_bytes: uint(row.get(6)?),
        at: DateTime::parse_from_rfc3339(&at).map_or(SystemTime::UNIX_EPOCH, SystemTime::from),
        collection: row.get::<_, Option<String>>(8)?.map(PathBuf::from),
        name: row.get(9)?,
    })
}

impl HistoryStore for HistoryDb {
    fn record(&self, entry: &HistoryEntry, request: &Request) -> Result<(), AppError> {
        let yaml = request_to_yaml(request)?;
        // fixed-width UTC text sorts in time order
        let at = DateTime::<Utc>::from(entry.at).to_rfc3339_opts(SecondsFormat::Millis, true);
        self.conn()
            .execute(
                "INSERT INTO history (method, url, status, status_text, time_ms, size_bytes, \
                 timestamp, collection_path, request_name, request_yaml) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    entry.method.as_str(),
                    entry.url,
                    entry.status,
                    entry.status_text,
                    int(entry.time_ms),
                    int(entry.size_bytes),
                    at,
                    entry.collection.as_ref().map(|p| p.display().to_string()),
                    entry.name,
                    yaml,
                ],
            )
            .map_err(db)?;
        Ok(())
    }

    fn recent(&self, limit: usize) -> Result<Vec<HistoryEntry>, AppError> {
        self.query(
            &format!("SELECT {COLUMNS} FROM history ORDER BY timestamp DESC, id DESC LIMIT ?1"),
            &[&(limit as i64)],
        )
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<HistoryEntry>, AppError> {
        self.query(
            &format!(
                "SELECT {COLUMNS} FROM history \
                 WHERE url LIKE ?1 ESCAPE '\\' OR method LIKE ?1 ESCAPE '\\' \
                 OR request_name LIKE ?1 ESCAPE '\\' \
                 ORDER BY timestamp DESC, id DESC LIMIT ?2"
            ),
            &[&like_pattern(query), &(limit as i64)],
        )
    }

    fn request(&self, id: i64) -> Result<Request, AppError> {
        let yaml: Option<String> = self
            .conn()
            .query_row(
                "SELECT request_yaml FROM history WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;
        request_from_yaml(&yaml.ok_or_else(|| AppError::History(format!("no entry {id}")))?)
    }

    fn clear(&self) -> Result<(), AppError> {
        self.conn().execute("DELETE FROM history", []).map_err(db)?;
        Ok(())
    }
}
