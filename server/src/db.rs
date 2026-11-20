use std::{
    path::Path,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, params};

use crate::{
    error::{ApiError, ApiResult},
    model::{Item, MediaType},
};

/// JSON blob tables next to the watchlist (same shape: key, data, updated_at).
#[derive(Clone, Copy)]
pub enum Table {
    Scans,
    Streams,
    Settings,
}

impl Table {
    fn name(self) -> &'static str {
        match self {
            Self::Scans => "scans",
            Self::Streams => "streams",
            Self::Settings => "settings",
        }
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// SQLite-backed store. Queries are tiny, so a mutex around one connection is enough.
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS watchlist (
                 type     TEXT    NOT NULL,
                 id       INTEGER NOT NULL,
                 data     TEXT    NOT NULL,
                 added_at INTEGER NOT NULL,
                 PRIMARY KEY (type, id)
             );
             CREATE TABLE IF NOT EXISTS scans    (key TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS streams  (key TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at INTEGER NOT NULL);",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn list(&self) -> ApiResult<Vec<Item>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT data FROM watchlist ORDER BY added_at DESC, rowid DESC")
            .map_err(db_err)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(db_err)?;
        let mut items = Vec::new();
        for row in rows {
            let data = row.map_err(db_err)?;
            if let Ok(item) = serde_json::from_str::<Item>(&data) {
                items.push(item);
            }
        }
        Ok(items)
    }

    pub fn put(&self, item: &Item) -> ApiResult<()> {
        let data = serde_json::to_string(item).map_err(|_| ApiError::internal("could not encode item"))?;
        let now = now_ms();
        self.lock()
            .execute(
                "INSERT INTO watchlist (type, id, data, added_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(type, id) DO UPDATE SET data = excluded.data",
                params![item.kind.as_str(), item.id as i64, data, now],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn delete(&self, kind: MediaType, id: u64) -> ApiResult<()> {
        self.lock()
            .execute("DELETE FROM watchlist WHERE type = ?1 AND id = ?2", params![kind.as_str(), id as i64])
            .map_err(db_err)?;
        Ok(())
    }

    pub fn blob_get(&self, table: Table, key: &str) -> ApiResult<Option<String>> {
        let sql = format!("SELECT data FROM {} WHERE key = ?1", table.name());
        match self.lock().query_row(&sql, params![key], |r| r.get::<_, String>(0)) {
            Ok(data) => Ok(Some(data)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(db_err(e)),
        }
    }

    pub fn blob_put(&self, table: Table, key: &str, data: &str) -> ApiResult<()> {
        let sql = format!(
            "INSERT INTO {} (key, data, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET data = excluded.data, updated_at = excluded.updated_at",
            table.name()
        );
        self.lock().execute(&sql, params![key, data, now_ms()]).map_err(db_err)?;
        Ok(())
    }

    pub fn blob_delete(&self, table: Table, key: &str) -> ApiResult<()> {
        let sql = format!("DELETE FROM {} WHERE key = ?1", table.name());
        self.lock().execute(&sql, params![key]).map_err(db_err)?;
        Ok(())
    }

    /// Drops rows not written since `before_ms`.
    pub fn blob_prune(&self, table: Table, before_ms: i64) -> ApiResult<()> {
        let sql = format!("DELETE FROM {} WHERE updated_at < ?1", table.name());
        self.lock().execute(&sql, params![before_ms]).map_err(db_err)?;
        Ok(())
    }
}

fn db_err(e: rusqlite::Error) -> ApiError {
    eprintln!("database error: {e}");
    ApiError::internal("database error")
}
