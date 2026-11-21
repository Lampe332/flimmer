//! Auto-download rules: per title on the watchlist a rule (quality, language, source, size). A
//! background job looks every 30 minutes for what is due (a series' newly aired episodes by TMDB
//! air date, a movie once), searches the torrents and queues the best release that fits the rule.

use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::Duration,
};

use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    api::{AppState, Params, St, TitleRef, media_type, numeric_id},
    db::{Table, now_ms},
    error::{ApiError, ApiResult},
    model::MediaType,
    progress,
    release::{self, Ranking, Torrent},
    scan::{self, iso},
    torrent::{self, Media, NewTorrent},
    tsearch,
};

pub const INTERVAL: Duration = Duration::from_secs(30 * 60);
const FIRST_RUN_AFTER: Duration = Duration::from_secs(120);
/// Episodes of one series queued per run (a backlog fills over several runs).
const MAX_PER_RULE: usize = 5;
const HEIGHTS: &[u32] = &[0, 480, 720, 1080, 2160];

#[derive(Default)]
pub struct JobState {
    last_run: AtomicI64,
    running: AtomicBool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    #[serde(rename = "type")]
    pub kind: MediaType,
    /// TMDB id.
    pub id: u64,
    pub title: String,
    pub year: Option<u32>,
    pub enabled: bool,
    /// 0 = any.
    pub min_height: u32,
    pub max_height: u32,
    /// "any" or a language code as in `Release.languages` ("de", "en" …).
    pub lang: String,
    /// Allowed `Release.source` values; empty = any (CAM/TS/screeners are never taken).
    pub sources: Vec<String>,
    /// 0 = no limit.
    pub max_size_gb: f64,
    pub min_seeders: u32,
    /// Series: episodes that aired on or after this day (YYYY-MM-DD).
    pub since: String,
    pub created_at: i64,
}

#[derive(Deserialize)]
struct RuleBody {
    title: String,
    year: Option<u32>,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    min_height: u32,
    #[serde(default)]
    max_height: u32,
    #[serde(default = "any")]
    lang: String,
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default)]
    max_size_gb: f64,
    #[serde(default = "three")]
    min_seeders: u32,
    since: Option<String>,
}

fn yes() -> bool {
    true
}

fn any() -> String {
    "any".into()
}

fn three() -> u32 {
    3
}

/// One release the job queued.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grab {
    /// "movie:<id>" | "tv:<id>:<season>:<episode>"
    pub key: String,
    #[serde(rename = "type")]
    pub kind: MediaType,
    pub id: u64,
    pub title: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub infohash: String,
    pub name: String,
    pub size: u64,
    pub seeders: u32,
    pub at: String,
}

/// What the job decided for one movie / episode.
#[derive(Debug, Serialize)]
pub struct Decision {
    rule: String,
    title: String,
    season: Option<u32>,
    episode: Option<u32>,
    air_date: Option<String>,
    /// "grab" | "none" (nothing fits) | "skip" (see `reason`)
    action: &'static str,
    /// English, for the log and the API docs.
    reason: Option<String>,
    /// The reason as a code the UI words itself (see `error::CODES`).
    reason_code: Option<&'static str>,
    /// Releases found before the rule was applied.
    found: usize,
    candidate: Option<Value>,
}

fn reason_text(code: &str) -> &'static str {
    match code {
        "not_on_watchlist" => "not on the watchlist",
        "tmdb_down" => "TMDB did not answer",
        "already_there" => "a torrent with it is already there",
        "next_run" => "more episodes are due than one run queues; next run",
        "search_failed" => "the search failed",
        "nothing_fits" => "no release fits the rule",
        "dry_run" => "dry run: nothing was queued",
        _ => "",
    }
}

fn rule_key(kind: MediaType, id: u64) -> String {
    format!("{}:{id}", kind.as_str())
}

fn grab_key(kind: MediaType, id: u64, episode: Option<(u32, u32)>) -> String {
    match episode {
        Some((s, e)) => format!("{}:{s}:{e}", rule_key(kind, id)),
        None => rule_key(kind, id),
    }
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn rules(st: &AppState) -> Vec<Rule> {
    let mut list: Vec<Rule> = st.db.blob_all(Table::Rules).unwrap_or_default().iter().filter_map(|d| serde_json::from_str(d).ok()).collect();
    list.sort_by_key(|r| std::cmp::Reverse(r.created_at));
    list
}

fn grabs(st: &AppState) -> Vec<Grab> {
    st.db.blob_all(Table::Grabs).unwrap_or_default().iter().filter_map(|d| serde_json::from_str(d).ok()).collect()
}

/// Whether a release fits the rule. Season packs never fit an episode.
pub fn fits(rule: &Rule, t: &Torrent, episode: bool) -> bool {
    let r = &t.release;
    let lang = match rule.lang.as_str() {
        "any" => true,
        // Releases without a language tag are English by scene convention.
        "en" => r.languages.is_empty() || r.languages.contains(&"en"),
        l => r.languages.contains(&l),
    };
    let height = r.height.unwrap_or(0);
    let usable = !(r.cam || r.three_d || matches!(r.dub, Some("mic" | "line")) || (episode && r.pack));
    lang && usable
        && t.seeders >= rule.min_seeders
        && (rule.min_height == 0 || height >= rule.min_height)
        && (rule.max_height == 0 || (height > 0 && height <= rule.max_height))
        && (rule.max_size_gb <= 0.0 || (t.size > 0 && t.size as f64 <= rule.max_size_gb * 1e9))
        && (rule.sources.is_empty() || r.source.is_some_and(|s| rule.sources.iter().any(|w| w.eq_ignore_ascii_case(s))))
}

/// The best release that fits the rule (quality first, the rule's language preferred).
pub fn pick(rule: &Rule, found: &[Torrent], episode: bool, saved: &scan::Filter) -> Option<Torrent> {
    let mut fitting: Vec<Torrent> = found.iter().filter(|t| fits(rule, t, episode)).cloned().collect();
    let filter = scan::Filter { min_height: rule.min_height, audio_lang: rule.lang.clone(), sort: saved.sort.clone() };
    release::rank(&mut fitting, &Ranking { filter: &filter, mode: Some("quality") });
    fitting.into_iter().next()
}

/// Episodes of the season `last.0` that count as newly aired: on or after `since`, not in the
/// future, not beyond TMDB's last aired episode, not watched. `(season, episode, air_date)`.
pub fn due(episodes: &[Value], last: (u32, u32), since: &str, today: &str, watched: &HashSet<(u32, u32)>) -> Vec<(u32, u32, String)> {
    let mut out: Vec<(u32, u32, String)> = episodes
        .iter()
        .filter_map(|x| {
            let n = u32::try_from(x.get("episode_number")?.as_u64()?).ok()?;
            let date = x.get("air_date")?.as_str()?.get(..10)?;
            (n <= last.1 && date >= since && date <= today && !watched.contains(&(last.0, n))).then(|| (last.0, n, date.to_owned()))
        })
        .collect();
    out.sort();
    out
}

/// Runs the job once. `dry` only reports what it would queue.
pub async fn run_once(st: &Arc<AppState>, dry: bool) -> Vec<Decision> {
    run_rules(st, dry, None).await
}

/// The job over all enabled rules, or (`preview`) a dry look at one rule that need not be stored
/// nor on the watchlist: nothing is queued or recorded for a preview.
async fn run_rules(st: &Arc<AppState>, dry: bool, preview: Option<Rule>) -> Vec<Decision> {
    let dry = dry || preview.is_some();
    let mut decisions = Vec::new();
    let watchlist: HashSet<(MediaType, u64)> = st.db.list().unwrap_or_default().iter().map(|i| (i.kind, i.id)).collect();
    let done: HashSet<String> = grabs(st).into_iter().map(|g| g.key).collect();
    let saved = scan::load_settings(st).map(|s| s.filter).unwrap_or(scan::Filter { min_height: 0, audio_lang: "any".into(), sort: "quality".into() });
    let today = iso(now_ms())[..10].to_owned();

    let list: Vec<Rule> = match &preview {
        Some(rule) => vec![rule.clone()],
        None => rules(st).into_iter().filter(|r| r.enabled).collect(),
    };
    for rule in list {
        let name = rule_key(rule.kind, rule.id);
        let decision = |episode: Option<(u32, u32, String)>, action, code: Option<&'static str>, found, candidate| Decision {
            rule: name.clone(),
            title: rule.title.clone(),
            season: episode.as_ref().map(|e| e.0),
            episode: episode.as_ref().map(|e| e.1),
            air_date: episode.map(|e| e.2),
            action,
            reason: code.map(|c| reason_text(c).to_owned()),
            reason_code: code,
            found,
            candidate,
        };
        if preview.is_none() && !watchlist.contains(&(rule.kind, rule.id)) {
            decisions.push(decision(None, "skip", Some("not_on_watchlist"), 0, None));
            continue;
        }
        let targets: Vec<Option<(u32, u32, String)>> = match rule.kind {
            MediaType::Movie => vec![None],
            MediaType::Tv => match due_episodes(st, &rule, &today).await {
                Ok(list) => list.into_iter().map(Some).collect(),
                Err(_) => {
                    decisions.push(decision(None, "skip", Some("tmdb_down"), 0, None));
                    continue;
                }
            },
        };
        let mut queued = 0;
        for target in targets {
            let episode = target.as_ref().map(|t| (t.0, t.1));
            let key = grab_key(rule.kind, rule.id, episode);
            if done.contains(&key) {
                continue;
            }
            if torrent::has_local(&st.torrents, rule.kind.as_str(), rule.id, episode) {
                decisions.push(decision(target, "skip", Some("already_there"), 0, None));
                continue;
            }
            if queued >= MAX_PER_RULE {
                decisions.push(decision(target, "skip", Some("next_run"), 0, None));
                continue;
            }
            let found = match tsearch::search_title(st, &TitleRef { kind: rule.kind, id: rule.id, episode }).await {
                Ok(f) => f,
                Err(_) => {
                    decisions.push(decision(target, "skip", Some("search_failed"), 0, None));
                    continue;
                }
            };
            let Some(best) = pick(&rule, &found.torrents, episode.is_some(), &saved) else {
                decisions.push(decision(target, "none", Some("nothing_fits"), found.torrents.len(), None));
                continue;
            };
            let candidate = json!({
                "infohash": best.infohash, "name": best.name, "size": best.size, "seeders": best.seeders,
                "height": best.release.height, "source": best.release.source, "languages": best.release.languages,
            });
            if !dry {
                let media = Media { kind: rule.kind.as_str().into(), id: Some(rule.id), title: rule.title.clone(), year: rule.year, season: episode.map(|e| e.0), episode: episode.map(|e| e.1) };
                let grab = Grab {
                    key: key.clone(),
                    kind: rule.kind,
                    id: rule.id,
                    title: rule.title.clone(),
                    season: episode.map(|e| e.0),
                    episode: episode.map(|e| e.1),
                    infohash: best.infohash.clone(),
                    name: best.name.clone(),
                    size: best.size,
                    seeders: best.seeders,
                    at: iso(now_ms()),
                };
                if let Ok(data) = serde_json::to_string(&grab) {
                    let _ = st.db.blob_put(Table::Grabs, &key, &data);
                }
                if !st.torrents.has(&best.infohash) {
                    let entry = torrent::add(
                        st,
                        NewTorrent {
                            id: best.infohash.clone(),
                            name: best.name.clone(),
                            mode: "download".into(),
                            media: Some(media),
                            trackers: best.trackers.clone(),
                            wanted: best.file_index.map(|i| vec![i]),
                            priority: 0,
                            auto: true,
                        },
                    );
                    torrent::emit(st, "grabbed", &entry, Some(format!("auto-download rule {name}")), None);
                }
                queued += 1;
            }
            decisions.push(decision(target, "grab", dry.then_some("dry_run"), found.torrents.len(), Some(candidate)));
        }
    }
    if !dry {
        st.autodl.last_run.store(now_ms(), Ordering::SeqCst);
    }
    decisions
}

async fn due_episodes(st: &AppState, rule: &Rule, today: &str) -> ApiResult<Vec<(u32, u32, String)>> {
    let Some(last) = progress::last_aired(st, rule.id).await? else { return Ok(Vec::new()) };
    let episodes = progress::season_episodes(st, rule.id, last.0).await?;
    let watched: HashSet<(u32, u32)> =
        st.db.progress_of(MediaType::Tv, rule.id)?.into_iter().filter(|r| r.finished).map(|r| (r.key.season, r.key.episode)).collect();
    Ok(due(&episodes, last, &rule.since, today, &watched))
}

/// The background job: first run two minutes after the start, then every 30 minutes.
pub fn spawn(st: Arc<AppState>) {
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_RUN_AFTER).await;
        loop {
            if !st.autodl.running.swap(true, Ordering::SeqCst) {
                run_once(&st, false).await;
                st.autodl.running.store(false, Ordering::SeqCst);
            }
            tokio::time::sleep(INTERVAL).await;
        }
    });
}

// ---------- API ----------

/// `GET /api/autodl/rules`
pub async fn list(State(st): St) -> ApiResult<Json<Value>> {
    let watchlist: HashSet<(MediaType, u64)> = st.db.list()?.iter().map(|i| (i.kind, i.id)).collect();
    let rules: Vec<Value> = rules(&st)
        .into_iter()
        .map(|r| {
            let on = watchlist.contains(&(r.kind, r.id));
            let mut v = serde_json::to_value(r).unwrap_or_default();
            v["on_watchlist"] = json!(on);
            v
        })
        .collect();
    let last = st.autodl.last_run.load(Ordering::SeqCst);
    Ok(Json(json!({ "rules": rules, "last_run": (last > 0).then(|| iso(last)), "interval_minutes": INTERVAL.as_secs() / 60 })))
}

/// `PUT /api/autodl/rules/:type/:id`
pub async fn put(State(st): St, Path((ty, id)): Path<(String, String)>, body: Bytes) -> ApiResult<StatusCode> {
    let rule = rule_from(&st, &ty, &id, &body)?;
    let data = serde_json::to_string(&rule).map_err(|_| ApiError::internal("could not encode the rule"))?;
    st.db.blob_put(Table::Rules, &rule_key(rule.kind, rule.id), &data)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/autodl/preview/:type/:id`: what a rule (the body of `PUT …/rules`) would take right
/// now. Nothing is stored and nothing is queued; the title need not be on the watchlist.
pub async fn preview(State(st): St, Path((ty, id)): Path<(String, String)>, body: Bytes) -> ApiResult<Json<Value>> {
    let rule = rule_from(&st, &ty, &id, &body)?;
    Ok(Json(json!({ "dry": true, "decisions": run_rules(&st, true, Some(rule)).await })))
}

/// Validates a rule body; an existing rule keeps its creation time and (unless given) its `since`.
fn rule_from(st: &AppState, ty: &str, id: &str, body: &[u8]) -> ApiResult<Rule> {
    let (kind, id) = (media_type(ty)?, numeric_id(id)?);
    let b: RuleBody = serde_json::from_slice(body).map_err(|e| ApiError::bad_request(format!("invalid body: {e}")))?;
    if b.title.trim().is_empty() {
        return Err(ApiError::bad_request("title is required"));
    }
    if !HEIGHTS.contains(&b.min_height) || !HEIGHTS.contains(&b.max_height) || (b.max_height > 0 && b.max_height < b.min_height) {
        return Err(ApiError::bad_request("min_height / max_height must be 0, 480, 720, 1080 or 2160 (max not below min)"));
    }
    let lang = b.lang.trim().to_lowercase();
    if lang.is_empty() || lang.len() > 8 || !lang.bytes().all(|c| c.is_ascii_lowercase()) {
        return Err(ApiError::bad_request("lang must be 'any' or a language code like 'de'"));
    }
    if !b.max_size_gb.is_finite() || b.max_size_gb < 0.0 {
        return Err(ApiError::bad_request("max_size_gb must be 0 (no limit) or positive"));
    }
    if b.since.as_deref().is_some_and(|s| !is_date(s)) {
        return Err(ApiError::bad_request("since must be a date (YYYY-MM-DD)"));
    }
    let key = rule_key(kind, id);
    let old: Option<Rule> = st.db.blob_get(Table::Rules, &key)?.and_then(|d| serde_json::from_str(&d).ok());
    let now = now_ms();
    let rule = Rule {
        kind,
        id,
        title: b.title.trim().to_owned(),
        year: b.year,
        enabled: b.enabled,
        min_height: b.min_height,
        max_height: b.max_height,
        lang,
        sources: b.sources.into_iter().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect(),
        max_size_gb: b.max_size_gb,
        min_seeders: b.min_seeders,
        since: b.since.or(old.as_ref().map(|o| o.since.clone())).unwrap_or_else(|| iso(now)[..10].to_owned()),
        created_at: old.map_or(now, |o| o.created_at),
    };
    Ok(rule)
}

/// `DELETE /api/autodl/rules/:type/:id`
pub async fn delete(State(st): St, Path((ty, id)): Path<(String, String)>) -> ApiResult<StatusCode> {
    st.db.blob_delete(Table::Rules, &rule_key(media_type(&ty)?, numeric_id(&id)?))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/autodl/history`
pub async fn history(State(st): St) -> Json<Vec<Grab>> {
    let mut list = grabs(&st);
    list.sort_by(|a, b| b.at.cmp(&a.at));
    Json(list)
}

/// `DELETE /api/autodl/history/:key`: forgets a grab, so the job may take that movie / episode again.
pub async fn history_delete(State(st): St, Path(key): Path<String>) -> ApiResult<StatusCode> {
    st.db.blob_delete(Table::Grabs, &key)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/autodl/run?dry=1`
pub async fn run(State(st): St, Query(q): Params) -> ApiResult<Json<Value>> {
    let dry = q.get("dry").is_some_and(|v| v == "1" || v == "true");
    if !dry && st.autodl.running.swap(true, Ordering::SeqCst) {
        return Err(ApiError::new(StatusCode::CONFLICT, "the auto-download job is running right now"));
    }
    let decisions = run_once(&st, dry).await;
    if !dry {
        st.autodl.running.store(false, Ordering::SeqCst);
    }
    Ok(Json(json!({ "dry": dry, "decisions": decisions })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::Hit;

    fn rule() -> Rule {
        Rule {
            kind: MediaType::Tv,
            id: 1,
            title: "Show".into(),
            year: None,
            enabled: true,
            min_height: 1080,
            max_height: 1080,
            lang: "de".into(),
            sources: vec!["WEB-DL".into(), "bluray".into()],
            max_size_gb: 8.0,
            min_seeders: 3,
            since: "2026-11-01".into(),
            created_at: 0,
        }
    }

    fn found() -> Vec<Torrent> {
        let hit = |hash: &str, name: &str, seeders: u32, gb: f64| Hit { source: "a", infohash: hash.into(), name: name.into(), seeders, size: (gb * 1e9) as u64, ..Hit::default() };
        release::merge(
            vec![
                hit("web", "Show.S01E02.German.DL.1080p.WEB.H264-AAA", 40, 2.0),
                hit("bluray", "Show.S01E02.German.DL.1080p.BluRay.x264-BBB", 12, 6.0),
                hit("big", "Show.S01E02.German.DL.1080p.BluRay.AVC.Remux-CCC", 90, 30.0),
                hit("uhd", "Show.S01E02.German.DL.2160p.WEB.H265-DDD", 500, 7.0),
                hit("english", "Show.S01E02.1080p.WEB.H264-EEE", 900, 2.0),
                hit("ts", "Show.S01E02.German.MD.TS.1080p.x264-FFF", 900, 1.0),
                hit("dead", "Show.S01E02.German.1080p.WEB.H264-GGG", 1, 2.0),
                hit("hdtv", "Show.S01E02.German.1080p.HDTV.x264-HHH", 300, 2.0),
                hit("pack", "Show.S01.German.DL.1080p.BluRay.x264-III", 700, 7.5),
            ],
            0,
        )
    }

    #[test]
    fn picks_the_best_release_that_fits_the_rule() {
        let saved = scan::Filter { min_height: 0, audio_lang: "any".into(), sort: "quality".into() };
        let list = found();
        let fitting: Vec<&str> = list.iter().filter(|t| fits(&rule(), t, true)).map(|t| t.infohash.as_str()).collect();
        assert_eq!(fitting, ["web", "bluray"]);
        // Both fit; the BluRay is the better source.
        assert_eq!(pick(&rule(), &list, true, &saved).unwrap().infohash, "bluray");
        // A season rule (no single episode) may take the pack.
        assert!(fits(&rule(), list.iter().find(|t| t.infohash == "pack").unwrap(), false));

        let english = Rule { lang: "en".into(), sources: Vec::new(), max_height: 0, ..rule() };
        assert_eq!(pick(&english, &list, true, &saved).unwrap().infohash, "uhd");
        let strict = Rule { max_size_gb: 1.0, ..rule() };
        assert!(pick(&strict, &list, true, &saved).is_none());
    }

    #[test]
    fn finds_newly_aired_episodes() {
        let eps: Vec<Value> = [(1, "2026-10-20"), (2, "2026-11-03"), (3, "2026-11-10"), (4, "2026-11-17"), (5, "2026-11-24")]
            .iter()
            .map(|(n, d)| json!({ "episode_number": n, "air_date": d }))
            .chain([json!({ "episode_number": 6, "air_date": null })])
            .collect();
        let watched: HashSet<(u32, u32)> = [(2, 3)].into();
        // Aired since the rule's date, up to today and TMDB's last aired episode, minus what was watched.
        let list = due(&eps, (2, 4), "2026-11-01", "2026-11-20", &watched);
        assert_eq!(list, vec![(2, 2, "2026-11-03".to_owned()), (2, 4, "2026-11-17".to_owned())]);
        assert!(due(&eps, (2, 4), "2026-11-18", "2026-11-20", &watched).is_empty());
        assert_eq!(grab_key(MediaType::Tv, 7, Some((2, 4))), "tv:7:2:4");
        assert_eq!(grab_key(MediaType::Movie, 45745, None), "movie:45745");
        assert!(is_date("2026-11-20") && !is_date("20.11.2026") && !is_date("2026-11-2x"));
    }
}
