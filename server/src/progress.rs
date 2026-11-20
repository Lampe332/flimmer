//! Watch progress, "Weiterschauen", history and "Neue Folgen".

use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
};
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    api::{AppState, Params, St, media_type, numeric_id, title_ref},
    db::{Key, now_ms},
    error::{ApiError, ApiResult},
    model::{self, Item, MediaType, WatchMeta},
    scan::iso,
    tmdb::TTL_SHOW,
};

/// Below this a position counts as "just opened": no history entry, no "Weiterschauen" for movies.
const MIN_WATCHED_S: f64 = 30.0;
const CONTINUE_MAX: usize = 20;
const HISTORY_MAX: u32 = 500;
const NEW_SERIES_MAX: usize = 60;
/// Episodes that aired within this many days count as new.
const NEW_WINDOW_DAYS: i64 = 30;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// ≥ 90 % watched, or less than 3 minutes left (only for titles of 10 minutes or more).
fn is_finished(position: f64, duration: f64) -> bool {
    duration > 0.0 && (position >= 0.9 * duration || (duration >= 600.0 && duration - position < 180.0))
}

fn key_of(ty: &str, id: &str, q: &HashMap<String, String>) -> ApiResult<Key> {
    let t = title_ref(ty, id, q)?;
    let (season, episode) = t.episode.unwrap_or((0, 0));
    Ok(Key { kind: t.kind, id: t.id, season, episode })
}

fn episode_of(key: &Key) -> (Option<u32>, Option<u32>) {
    match key.kind {
        MediaType::Movie => (None, None),
        MediaType::Tv => (Some(key.season), Some(key.episode)),
    }
}

fn meta_from(body: &[u8], key: &Key) -> ApiResult<WatchMeta> {
    let mut meta: WatchMeta =
        serde_json::from_slice(body).map_err(|e| ApiError::bad_request(format!("invalid body: {e}")))?;
    // The path is authoritative for the title.
    meta.item.kind = key.kind;
    meta.item.id = key.id;
    Ok(meta)
}

// ---------- progress ----------

#[derive(Serialize)]
pub struct Progress {
    #[serde(rename = "type")]
    kind: MediaType,
    id: u64,
    season: Option<u32>,
    episode: Option<u32>,
    position: f64,
    duration: f64,
    finished: bool,
    updated_at: String,
}

pub async fn of_title(State(st): St, Path((ty, id)): Path<(String, String)>) -> ApiResult<Json<Vec<Progress>>> {
    let kind = media_type(&ty)?;
    let id = numeric_id(&id)?;
    let rows = st.db.progress_of(kind, id)?;
    Ok(Json(
        rows.into_iter()
            .map(|r| {
                let (season, episode) = episode_of(&r.key);
                Progress {
                    kind: r.key.kind,
                    id: r.key.id,
                    season,
                    episode,
                    position: r.position,
                    duration: r.duration,
                    finished: r.finished,
                    updated_at: iso(r.updated_at),
                }
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct ProgressBody {
    position: f64,
    duration: f64,
    #[serde(flatten)]
    meta: WatchMeta,
}

pub async fn put(
    State(st): St,
    Path((ty, id)): Path<(String, String)>,
    Query(q): Params,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let key = key_of(&ty, &id, &q)?;
    let mut b: ProgressBody =
        serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(format!("invalid body: {e}")))?;
    let valid = |v: f64| v.is_finite() && (0.0..=172_800.0).contains(&v);
    if !valid(b.position) || !valid(b.duration) {
        return Err(ApiError::bad_request("'position' and 'duration' must be seconds between 0 and 48 h"));
    }
    b.meta.item.kind = key.kind;
    b.meta.item.id = key.id;
    let position = if b.duration > 0.0 { b.position.min(b.duration) } else { b.position };
    let finished = is_finished(position, b.duration);
    st.db.progress_put(&key, position, b.duration, finished, &b.meta)?;
    if finished || position >= MIN_WATCHED_S {
        st.db.history_touch(&key, position, b.duration, finished, &b.meta)?;
    }
    if key.kind == MediaType::Tv {
        // Playing from a season pack: get the next episode's file ready for auto-next.
        tokio::spawn(crate::torrent::on_progress(st.clone(), key.id, key.season, key.episode, position, b.duration));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn watched(
    State(st): St,
    Path((ty, id)): Path<(String, String)>,
    Query(q): Params,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let key = key_of(&ty, &id, &q)?;
    let meta = meta_from(&body, &key)?;
    st.db.progress_mark_watched(&key, &meta)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unwatched(State(st): St, Path((ty, id)): Path<(String, String)>, Query(q): Params) -> ApiResult<StatusCode> {
    let key = key_of(&ty, &id, &q)?;
    st.db.progress_delete(&key)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- TMDB show data (6 h cache) ----------

struct Show {
    /// (season_number, episode_count), without specials.
    seasons: Vec<(u32, u32)>,
    /// (season, episode, air_date) of the newest aired episode.
    last_aired: Option<(u32, u32, String)>,
}

fn num(v: &Value, key: &str) -> Option<u32> {
    v.get(key).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok())
}

async fn show(st: &AppState, id: u64) -> ApiResult<Show> {
    let v = st.tmdb.get(&format!("/tv/{id}"), &[], Some(TTL_SHOW)).await?;
    let seasons = v
        .get("seasons")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|s| Some((num(s, "season_number")?, num(s, "episode_count").unwrap_or(0))))
                .filter(|(n, _)| *n != 0)
                .collect()
        })
        .unwrap_or_default();
    let last_aired = v.get("last_episode_to_air").and_then(|e| {
        let date = e.get("air_date").and_then(Value::as_str).filter(|d| d.len() >= 10)?;
        Some((num(e, "season_number")?, num(e, "episode_number")?, date[..10].to_owned()))
    });
    Ok(Show { seasons, last_aired })
}

/// (season, episode) of the newest aired episode of a series.
pub(crate) async fn last_aired(st: &AppState, id: u64) -> ApiResult<Option<(u32, u32)>> {
    Ok(show(st, id).await?.last_aired.map(|(s, e, _)| (s, e)))
}

pub(crate) async fn season_episodes(st: &AppState, id: u64, season: u32) -> ApiResult<Vec<Value>> {
    let v = st.tmdb.get(&format!("/tv/{id}/season/{season}"), &[], Some(TTL_SHOW)).await?;
    Ok(v.get("episodes").and_then(Value::as_array).cloned().unwrap_or_default())
}

impl Show {
    /// The episode after `(s, e)`, if it has aired already.
    fn next(&self, s: u32, e: u32) -> Option<(u32, u32)> {
        if s == 0 {
            return None;
        }
        let count = self.seasons.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)?;
        let cand = if e < count {
            (s, e + 1)
        } else {
            let n = self.seasons.iter().filter(|(n, c)| *n > s && *c > 0).map(|(n, _)| *n).min()?;
            (n, 1)
        };
        let (ls, le, _) = self.last_aired.as_ref()?;
        (cand <= (*ls, *le)).then_some(cand)
    }
}

// ---------- continue watching ----------

#[derive(Serialize)]
pub struct ContinueEntry {
    item: Item,
    season: Option<u32>,
    episode: Option<u32>,
    episode_name: Option<String>,
    still: Option<String>,
    position: f64,
    duration: f64,
    up_next: bool,
    updated_at: String,
}

pub async fn continue_list(State(st): St) -> ApiResult<Json<Vec<ContinueEntry>>> {
    let rows = st.db.progress_all()?;
    let by_key: HashMap<Key, (f64, f64, bool)> =
        rows.iter().map(|r| (r.key, (r.position, r.duration, r.finished))).collect();
    let mut seen = HashSet::new();
    let latest: Vec<_> = rows
        .iter()
        .filter(|r| seen.insert((r.key.kind, r.key.id)))
        .filter(|r| !r.hidden)
        .take(CONTINUE_MAX * 2)
        .collect();

    let (app, by_key): (&AppState, _) = (&st, &by_key);
    let entries = join_all(latest.into_iter().map(|r| async move {
        let base = ContinueEntry {
            item: r.meta.item.clone(),
            season: None,
            episode: None,
            episode_name: r.meta.episode_name.clone(),
            still: r.meta.still.clone(),
            position: r.position,
            duration: r.duration,
            up_next: false,
            updated_at: iso(r.updated_at),
        };
        match r.key.kind {
            MediaType::Movie => (!r.finished && r.position >= MIN_WATCHED_S).then_some(base),
            MediaType::Tv if !r.finished => Some(ContinueEntry {
                season: Some(r.key.season),
                episode: Some(r.key.episode),
                // barely started: show it as the next episode rather than at 0 %
                up_next: r.position < MIN_WATCHED_S,
                ..base
            }),
            MediaType::Tv => {
                let show = show(app, r.key.id).await.ok()?;
                let (mut s, mut e) = show.next(r.key.season, r.key.episode)?;
                // skip episodes that are already marked as watched
                for _ in 0..50 {
                    match by_key.get(&Key { season: s, episode: e, ..r.key }) {
                        Some((_, _, true)) => (s, e) = show.next(s, e)?,
                        _ => break,
                    }
                }
                let own = by_key.get(&Key { season: s, episode: e, ..r.key });
                let (position, duration) = own.map(|(p, d, _)| (*p, *d)).unwrap_or((0.0, 0.0));
                let eps = season_episodes(app, r.key.id, s).await.unwrap_or_default();
                let ep = eps.iter().find(|x| num(x, "episode_number") == Some(e));
                Some(ContinueEntry {
                    season: Some(s),
                    episode: Some(e),
                    episode_name: ep.and_then(|x| x.get("name")).and_then(Value::as_str).map(str::to_owned),
                    still: ep.and_then(|x| model::image("w300", x, "still_path")),
                    position,
                    duration,
                    up_next: position < MIN_WATCHED_S,
                    ..base
                })
            }
        }
    }))
    .await;
    Ok(Json(entries.into_iter().flatten().take(CONTINUE_MAX).collect()))
}

pub async fn continue_hide(State(st): St, Path((ty, id)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let kind = media_type(&ty)?;
    let id = numeric_id(&id)?;
    st.db.progress_hide(kind, id)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- history ----------

#[derive(Serialize)]
pub struct HistoryEntry {
    entry: i64,
    item: Item,
    season: Option<u32>,
    episode: Option<u32>,
    episode_name: Option<String>,
    still: Option<String>,
    position: f64,
    duration: f64,
    finished: bool,
    watched_at: String,
}

pub async fn history(State(st): St) -> ApiResult<Json<Vec<HistoryEntry>>> {
    let rows = st.db.history_list(HISTORY_MAX)?;
    Ok(Json(
        rows.into_iter()
            .map(|r| {
                let (season, episode) = episode_of(&r.key);
                HistoryEntry {
                    entry: r.entry,
                    item: r.meta.item,
                    season,
                    episode,
                    episode_name: r.meta.episode_name,
                    still: r.meta.still,
                    position: r.position,
                    duration: r.duration,
                    finished: r.finished,
                    watched_at: iso(r.watched_at),
                }
            })
            .collect(),
    ))
}

pub async fn history_delete(State(st): St, Path(entry): Path<String>) -> ApiResult<StatusCode> {
    let entry = numeric_id(&entry)?;
    st.db.history_delete(entry as i64)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn history_clear(State(st): St) -> ApiResult<StatusCode> {
    st.db.history_clear()?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- new episodes ----------

#[derive(Serialize)]
pub struct NewEpisode {
    item: Item,
    season: u32,
    episode: u32,
    name: String,
    still: Option<String>,
    air_date: String,
    /// New episodes of this series, this one included.
    count: usize,
}

pub async fn new_episodes(State(st): St) -> ApiResult<Json<Vec<NewEpisode>>> {
    // Series from the history (newest first, with the furthest episode seen), then the watchlist.
    let mut order: Vec<u64> = Vec::new();
    let mut series: HashMap<u64, (Item, (u32, u32))> = HashMap::new();
    for r in st.db.progress_all()?.into_iter().filter(|r| r.key.kind == MediaType::Tv) {
        let here = (r.key.season, r.key.episode);
        series
            .entry(r.key.id)
            .and_modify(|(_, furthest)| *furthest = (*furthest).max(here))
            .or_insert_with(|| {
                order.push(r.key.id);
                (r.meta.item.clone(), here)
            });
    }
    for item in st.db.list()?.into_iter().filter(|i| i.kind == MediaType::Tv) {
        if let std::collections::hash_map::Entry::Vacant(slot) = series.entry(item.id) {
            order.push(item.id);
            slot.insert((item, (0, 0)));
        }
    }
    order.truncate(NEW_SERIES_MAX);

    let now = now_ms();
    let today = iso(now)[..10].to_owned();
    let from = iso(now - NEW_WINDOW_DAYS * DAY_MS)[..10].to_owned();
    let found = join_all(order.into_iter().filter_map(|id| series.remove(&id).map(|s| (id, s))).map(
        |(id, (item, after))| {
            let (st, today, from): (&AppState, _, _) = (&st, &today, &from);
            async move {
                let show = show(st, id).await.ok()?;
                let (ls, le, date) = show.last_aired?;
                if date < *from {
                    return None;
                }
                let eps = season_episodes(st, id, ls).await.ok()?;
                let fresh: Vec<(u32, &Value, &str)> = eps
                    .iter()
                    .filter_map(|x| {
                        let n = num(x, "episode_number")?;
                        let d = x.get("air_date").and_then(Value::as_str)?;
                        (n <= le && (ls, n) > after && d >= from.as_str() && d <= today.as_str()).then_some((n, x, d))
                    })
                    .collect();
                let (n, first, d) = *fresh.iter().min_by_key(|(n, _, _)| *n)?;
                Some(NewEpisode {
                    item,
                    season: ls,
                    episode: n,
                    name: first.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    still: model::image("w300", first, "still_path"),
                    air_date: d.to_owned(),
                    count: fresh.len(),
                })
            }
        },
    ))
    .await;
    let mut out: Vec<NewEpisode> = found.into_iter().flatten().collect();
    out.sort_by(|a, b| b.air_date.cmp(&a.air_date));
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finished_rule() {
        assert!(!is_finished(0.0, 0.0));
        assert!(!is_finished(600.0, 2700.0));
        assert!(is_finished(2430.0, 2700.0)); // 90 %
        assert!(is_finished(2530.0, 2700.0)); // < 3 min left
        assert!(!is_finished(10.0, 150.0)); // short clip: only the 90 % rule
        assert!(is_finished(140.0, 150.0));
    }

    #[test]
    fn next_episode() {
        let show = Show { seasons: vec![(1, 3), (2, 2), (3, 8)], last_aired: Some((3, 2, "2026-11-01".into())) };
        assert_eq!(show.next(1, 1), Some((1, 2)));
        assert_eq!(show.next(1, 3), Some((2, 1)));
        assert_eq!(show.next(3, 1), Some((3, 2)));
        assert_eq!(show.next(3, 2), None); // not aired yet
        assert_eq!(show.next(0, 1), None);
    }
}
