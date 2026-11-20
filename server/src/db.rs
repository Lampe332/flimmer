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

/// SQLite-backed watchlist. Queries are tiny, so a mutex around one connection is enough.
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
             );",
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
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
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
}

fn db_err(e: rusqlite::Error) -> ApiError {
    eprintln!("database error: {e}");
    ApiError::internal("database error")
}
