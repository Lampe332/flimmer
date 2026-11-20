use std::{
    collections::HashMap,
    path::Path,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, params};

use crate::{
    error::{ApiError, ApiResult},
    model::{Item, MediaType, WatchMeta},
};

/// JSON blob tables next to the watchlist (same shape: key, data, updated_at).
#[derive(Clone, Copy)]
pub enum Table {
    Scans,
    Streams,
    Settings,
    Torrents,
    /// Torrent notifications (finished / failed / grabbed).
    Events,
    /// Auto-download rules, keyed `"<type>:<tmdb id>"`.
    Rules,
    /// What the auto-download job queued, keyed like a rule plus `":<season>:<episode>"` for episodes.
    Grabs,
}

impl Table {
    fn name(self) -> &'static str {
        match self {
            Self::Scans => "scans",
            Self::Streams => "streams",
            Self::Settings => "settings",
            Self::Torrents => "torrents",
            Self::Events => "torrent_events",
            Self::Rules => "autodl_rules",
            Self::Grabs => "autodl_grabs",
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
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS torrents (key TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS torrent_events (key TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS autodl_rules (key TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS autodl_grabs (key TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS progress (
                 type       TEXT    NOT NULL,
                 id         INTEGER NOT NULL,
                 season     INTEGER NOT NULL,
                 episode    INTEGER NOT NULL,
                 position   REAL    NOT NULL,
                 duration   REAL    NOT NULL,
                 finished   INTEGER NOT NULL,
                 hidden     INTEGER NOT NULL DEFAULT 0,
                 data       TEXT    NOT NULL,
                 updated_at INTEGER NOT NULL,
                 PRIMARY KEY (type, id, season, episode)
             );
             CREATE TABLE IF NOT EXISTS history (
                 entry      INTEGER PRIMARY KEY AUTOINCREMENT,
                 type       TEXT    NOT NULL,
                 id         INTEGER NOT NULL,
                 season     INTEGER NOT NULL,
                 episode    INTEGER NOT NULL,
                 position   REAL    NOT NULL,
                 duration   REAL    NOT NULL,
                 finished   INTEGER NOT NULL,
                 data       TEXT    NOT NULL,
                 watched_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS history_key ON history (type, id, season, episode, watched_at);
             CREATE TABLE IF NOT EXISTS provider_health (
                 key         TEXT    PRIMARY KEY,
                 ok          INTEGER NOT NULL,
                 fail        INTEGER NOT NULL,
                 fail_streak INTEGER NOT NULL,
                 avg_ms      INTEGER,
                 last_ok     INTEGER,
                 last_try    INTEGER NOT NULL,
                 last_error  INTEGER NOT NULL,
                 last_reason TEXT
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

    pub fn blob_all(&self, table: Table) -> ApiResult<Vec<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!("SELECT data FROM {} ORDER BY rowid", table.name())).map_err(db_err)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(db_err)?;
        rows.collect::<Result<_, _>>().map_err(db_err)
    }

    /// Drops rows not written since `before_ms`.
    pub fn blob_prune(&self, table: Table, before_ms: i64) -> ApiResult<()> {
        let sql = format!("DELETE FROM {} WHERE updated_at < ?1", table.name());
        self.lock().execute(&sql, params![before_ms]).map_err(db_err)?;
        Ok(())
    }

    // ---------- watch progress ----------

    /// Saves a playback position; a save always brings the title back into "Weiterschauen".
    pub fn progress_put(&self, key: &Key, position: f64, duration: f64, finished: bool, meta: &WatchMeta) -> ApiResult<()> {
        let data = encode(meta)?;
        self.lock()
            .execute(
                "INSERT INTO progress (type, id, season, episode, position, duration, finished, hidden, data, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9)
                 ON CONFLICT(type, id, season, episode) DO UPDATE SET
                     position = excluded.position, duration = excluded.duration, finished = excluded.finished,
                     hidden = 0, data = excluded.data, updated_at = excluded.updated_at",
                params![key.kind.as_str(), key.id as i64, key.season, key.episode, position, duration, finished, data, now_ms()],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// "Als gesehen markieren": finished, position at the end.
    pub fn progress_mark_watched(&self, key: &Key, meta: &WatchMeta) -> ApiResult<()> {
        let data = encode(meta)?;
        self.lock()
            .execute(
                "INSERT INTO progress (type, id, season, episode, position, duration, finished, hidden, data, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, 0, 1, 0, ?5, ?6)
                 ON CONFLICT(type, id, season, episode) DO UPDATE SET
                     position = progress.duration, finished = 1, data = excluded.data, updated_at = excluded.updated_at",
                params![key.kind.as_str(), key.id as i64, key.season, key.episode, data, now_ms()],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn progress_delete(&self, key: &Key) -> ApiResult<()> {
        self.lock()
            .execute(
                "DELETE FROM progress WHERE type = ?1 AND id = ?2 AND season = ?3 AND episode = ?4",
                params![key.kind.as_str(), key.id as i64, key.season, key.episode],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Removes a title from "Weiterschauen" without forgetting its positions.
    pub fn progress_hide(&self, kind: MediaType, id: u64) -> ApiResult<()> {
        self.lock()
            .execute("UPDATE progress SET hidden = 1 WHERE type = ?1 AND id = ?2", params![kind.as_str(), id as i64])
            .map_err(db_err)?;
        Ok(())
    }

    /// All rows of one title, in episode order.
    pub fn progress_of(&self, kind: MediaType, id: u64) -> ApiResult<Vec<ProgressRow>> {
        self.progress_query(
            "SELECT type, id, season, episode, position, duration, finished, hidden, data, updated_at
             FROM progress WHERE type = ?1 AND id = ?2 ORDER BY season, episode",
            params![kind.as_str(), id as i64],
        )
    }

    /// Every row, most recently watched first.
    pub fn progress_all(&self) -> ApiResult<Vec<ProgressRow>> {
        self.progress_query(
            "SELECT type, id, season, episode, position, duration, finished, hidden, data, updated_at
             FROM progress ORDER BY updated_at DESC",
            params![],
        )
    }

    fn progress_query(&self, sql: &str, args: &[&dyn rusqlite::ToSql]) -> ApiResult<Vec<ProgressRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(sql).map_err(db_err)?;
        let rows = stmt
            .query_map(args, |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, u32>(2)?,
                    r.get::<_, u32>(3)?,
                    r.get::<_, f64>(4)?,
                    r.get::<_, f64>(5)?,
                    r.get::<_, bool>(6)?,
                    r.get::<_, bool>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, i64>(9)?,
                ))
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (kind, id, season, episode, position, duration, finished, hidden, data, updated_at) = row.map_err(db_err)?;
            let (Some(kind), Ok(meta)) = (MediaType::parse(&kind), serde_json::from_str::<WatchMeta>(&data)) else {
                continue;
            };
            out.push(ProgressRow {
                key: Key { kind, id: id as u64, season, episode },
                position,
                duration,
                finished,
                hidden,
                meta,
                updated_at,
            });
        }
        Ok(out)
    }

    // ---------- history ----------

    /// Records watching in the history. Watching the same movie/episode again within
    /// `SESSION_MS` updates that entry instead of adding a new one.
    pub fn history_touch(&self, key: &Key, position: f64, duration: f64, finished: bool, meta: &WatchMeta) -> ApiResult<()> {
        const SESSION_MS: i64 = 6 * 60 * 60 * 1000;
        let data = encode(meta)?;
        let now = now_ms();
        let conn = self.lock();
        let recent = conn.query_row(
            "SELECT entry FROM history WHERE type = ?1 AND id = ?2 AND season = ?3 AND episode = ?4 AND watched_at > ?5
             ORDER BY watched_at DESC LIMIT 1",
            params![key.kind.as_str(), key.id as i64, key.season, key.episode, now - SESSION_MS],
            |r| r.get::<_, i64>(0),
        );
        match recent {
            Ok(entry) => conn.execute(
                "UPDATE history SET position = ?1, duration = ?2, finished = MAX(finished, ?3), data = ?4, watched_at = ?5
                 WHERE entry = ?6",
                params![position, duration, finished, data, now, entry],
            ),
            Err(rusqlite::Error::QueryReturnedNoRows) => conn.execute(
                "INSERT INTO history (type, id, season, episode, position, duration, finished, data, watched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![key.kind.as_str(), key.id as i64, key.season, key.episode, position, duration, finished, data, now],
            ),
            Err(e) => Err(e),
        }
        .map_err(db_err)?;
        Ok(())
    }

    /// Newest first.
    pub fn history_list(&self, limit: u32) -> ApiResult<Vec<HistoryRow>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT entry, type, id, season, episode, position, duration, finished, data, watched_at
                 FROM history ORDER BY watched_at DESC, entry DESC LIMIT ?1",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![limit], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, u32>(3)?,
                    r.get::<_, u32>(4)?,
                    r.get::<_, f64>(5)?,
                    r.get::<_, f64>(6)?,
                    r.get::<_, bool>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, i64>(9)?,
                ))
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (entry, kind, id, season, episode, position, duration, finished, data, watched_at) = row.map_err(db_err)?;
            let (Some(kind), Ok(meta)) = (MediaType::parse(&kind), serde_json::from_str::<WatchMeta>(&data)) else {
                continue;
            };
            out.push(HistoryRow {
                entry,
                key: Key { kind, id: id as u64, season, episode },
                position,
                duration,
                finished,
                meta,
                watched_at,
            });
        }
        Ok(out)
    }

    pub fn history_delete(&self, entry: i64) -> ApiResult<()> {
        self.lock().execute("DELETE FROM history WHERE entry = ?1", params![entry]).map_err(db_err)?;
        Ok(())
    }

    pub fn history_clear(&self) -> ApiResult<()> {
        self.lock().execute("DELETE FROM history", []).map_err(db_err)?;
        Ok(())
    }

    // ---------- provider health (scan order and skipping) ----------

    /// All rows whose key starts with `prefix` (`"movie:"` / `"tv:"`), keyed by the rest (provider id).
    pub fn health_all(&self, prefix: &str) -> ApiResult<HashMap<String, Health>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT key, fail_streak, avg_ms, last_try, last_error, last_reason
                 FROM provider_health WHERE substr(key, 1, ?1) = ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![prefix.len() as i64, prefix], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    Health {
                        fail_streak: r.get(1)?,
                        avg_ms: r.get(2)?,
                        last_try: r.get(3)?,
                        last_error: r.get(4)?,
                        last_reason: r.get(5)?,
                    },
                ))
            })
            .map_err(db_err)?;
        let mut out = HashMap::new();
        for row in rows {
            let (key, health) = row.map_err(db_err)?;
            out.insert(key[prefix.len()..].to_string(), health);
        }
        Ok(out)
    }

    /// Adds the outcomes of one scan, keyed `"<movie|tv>:<provider id>"`.
    pub fn health_record(&self, outcomes: &[(String, Outcome)]) -> ApiResult<()> {
        let now = now_ms();
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(db_err)?;
        for (key, outcome) in outcomes {
            match outcome {
                Ok(ms) => tx.execute(
                    "INSERT INTO provider_health (key, ok, fail, fail_streak, avg_ms, last_ok, last_try, last_error)
                     VALUES (?1, 1, 0, 0, ?2, ?3, ?3, 0)
                     ON CONFLICT(key) DO UPDATE SET
                         ok = ok + 1, fail_streak = 0, avg_ms = (COALESCE(avg_ms, ?2) * 2 + ?2) / 3,
                         last_ok = ?3, last_try = ?3, last_error = 0, last_reason = NULL",
                    params![key, *ms as i64, now],
                ),
                Err((is_error, reason)) => tx.execute(
                    "INSERT INTO provider_health (key, ok, fail, fail_streak, last_try, last_error, last_reason)
                     VALUES (?1, 0, 1, 1, ?2, ?3, ?4)
                     ON CONFLICT(key) DO UPDATE SET
                         fail = fail + 1, fail_streak = fail_streak + 1, last_try = ?2, last_error = ?3, last_reason = ?4",
                    params![key, now, is_error, reason],
                ),
            }
            .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)
    }
}

/// One scan outcome of a provider: `Ok(ms)` = stream found after `ms`, `Err((is_error, reason))` = none
/// (`is_error`: the page was unreachable, not just without a stream).
pub type Outcome = Result<u64, (bool, Option<String>)>;

/// What the scans so far say about one provider (per media type). The table also counts `ok` / `fail`
/// and keeps `last_ok` for looking at it by hand.
#[derive(Debug, Clone, Default)]
pub struct Health {
    /// Scans without a stream since the last one with a stream.
    pub fail_streak: u32,
    /// Smoothed time until the stream was captured.
    pub avg_ms: Option<i64>,
    pub last_try: i64,
    /// The last failure was an unreachable page (`error`), not a page without stream (`no_stream`).
    pub last_error: bool,
    pub last_reason: Option<String>,
}

/// A movie (`season = episode = 0`) or one episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub kind: MediaType,
    pub id: u64,
    pub season: u32,
    pub episode: u32,
}

#[derive(Debug, Clone)]
pub struct ProgressRow {
    pub key: Key,
    pub position: f64,
    pub duration: f64,
    pub finished: bool,
    pub hidden: bool,
    pub meta: WatchMeta,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub entry: i64,
    pub key: Key,
    pub position: f64,
    pub duration: f64,
    pub finished: bool,
    pub meta: WatchMeta,
    pub watched_at: i64,
}

fn encode(meta: &WatchMeta) -> ApiResult<String> {
    serde_json::to_string(meta).map_err(|_| ApiError::internal("could not encode item"))
}

fn db_err(e: rusqlite::Error) -> ApiError {
    eprintln!("database error: {e}");
    ApiError::internal("database error")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_counts_streaks_per_media_type() {
        let db = Db::open(Path::new(":memory:")).expect("in-memory db");
        let rec = |key: &str, outcome: Outcome| db.health_record(&[(key.to_string(), outcome)]).unwrap();
        rec("movie:a", Ok(900));
        rec("movie:a", Ok(300));
        rec("movie:b", Err((false, None)));
        db.health_record(&[
            ("movie:b".to_string(), Err((true, Some("provider host not found".to_string())))),
            ("tv:a".to_string(), Err((false, Some("hint".to_string())))),
        ])
        .unwrap();

        let movie = db.health_all("movie:").unwrap();
        assert_eq!(movie.len(), 2);
        assert_eq!((movie["a"].fail_streak, movie["a"].avg_ms), (0, Some(700)));
        assert_eq!((movie["b"].fail_streak, movie["b"].last_error), (2, true));
        assert_eq!(movie["b"].last_reason.as_deref(), Some("provider host not found"));

        // a stream ends the streak and clears the reason
        rec("movie:b", Ok(1200));
        let b = &db.health_all("movie:").unwrap()["b"];
        assert_eq!((b.fail_streak, b.avg_ms, b.last_error, b.last_reason.as_deref()), (0, Some(1200), false, None));

        let tv = db.health_all("tv:").unwrap();
        assert_eq!((tv.len(), tv["a"].fail_streak), (1, 1));
    }
}
