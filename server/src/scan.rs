//! Provider scan (through the injected sniffer), capabilities and settings.

use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

use crate::{
    api::{self, AppState, Source},
    db::{Health, Outcome, Table, now_ms},
    error::{ApiError, ApiResult},
    hls::{self, Track, Variant},
    sniff::{Candidate, CandidateKind, Sniffer},
    stream::{self, StreamKind, StreamRow},
};

const REUSE_MS: i64 = 6 * 60 * 60 * 1000;
/// Pages loading at the same time (default; the setting allows 4..=12). More finish a scan sooner, but pages
/// that normally have a stream then fail to start more often (tried: 8, 10, 12; see docs/scan-speed.md).
const PARALLEL: usize = 10;
const PARALLEL_MIN: usize = 4;
const PARALLEL_MAX: usize = 12;
/// The sniffer gives up on its own after at most ~25 s; this only guards against one that hangs.
const PROBE_LIMIT: Duration = Duration::from_secs(32);
/// A provider without a stream in this many scans in a row is skipped ...
const SKIP_AFTER: u32 = 3;
/// ... and tried again once this much time has passed since its last try.
const RETEST_MS: i64 = 24 * 60 * 60 * 1000;
/// After something less than a master playlist showed up (media playlist, plain mp4, DASH
/// manifest), wait this long for a master before settling for what there is.
const MP4_GRACE: Duration = Duration::from_secs(3);
/// Init/media segments are `video/mp4` too; a real film file is far bigger than one of those.
const MP4_MIN_BYTES: u64 = 30 * 1024 * 1024;
/// Request headers of the page that the upstream may check.
const KEEP_HEADERS: [&str; 5] = ["referer", "origin", "user-agent", "cookie", "authorization"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Pending,
    Running,
    Ok,
    NoStream,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScanResult {
    provider_id: String,
    name: String,
    status: Status,
    ms: Option<u64>,
    max_width: Option<u32>,
    max_height: Option<u32>,
    max_bandwidth: Option<u64>,
    variants: Vec<Variant>,
    audio: Vec<Track>,
    subtitles: Vec<Track>,
    stream_id: Option<String>,
    kind: Option<StreamKind>,
    error: Option<String>,
}

impl ScanResult {
    fn pending(src: &Source) -> Self {
        Self {
            provider_id: src.id.clone(),
            name: src.name.clone(),
            status: Status::Pending,
            ms: None,
            max_width: None,
            max_height: None,
            max_bandwidth: None,
            variants: Vec::new(),
            audio: Vec::new(),
            subtitles: Vec::new(),
            stream_id: None,
            kind: None,
            error: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct ScanData {
    #[serde(skip)]
    running: bool,
    started_at: i64,
    finished_at: Option<i64>,
    results: Vec<ScanResult>,
}

impl ScanData {
    fn body(&self) -> Value {
        json!({
            "state": if self.running { "running" } else { "done" },
            "started_at": iso(self.started_at),
            "finished_at": self.finished_at.map(iso),
            "results": self.results,
        })
    }
}

type Shared = Arc<Mutex<ScanData>>;

pub struct Scans {
    map: Mutex<HashMap<String, Shared>>,
    /// `PARALLEL_MAX` permits; the ones above the setting are parked, so a change never cuts running pages short.
    slots: Arc<Semaphore>,
    parked: Arc<Mutex<Vec<OwnedSemaphorePermit>>>,
    want_parked: Arc<std::sync::atomic::AtomicUsize>,
}

impl Default for Scans {
    fn default() -> Self {
        let slots = Arc::new(Semaphore::new(PARALLEL_MAX));
        let parked = (0..PARALLEL_MAX - PARALLEL).filter_map(|_| slots.clone().try_acquire_owned().ok()).collect();
        Self {
            map: Mutex::new(HashMap::new()),
            slots,
            parked: Arc::new(Mutex::new(parked)),
            want_parked: Arc::new((PARALLEL_MAX - PARALLEL).into()),
        }
    }
}

impl Scans {
    /// Pages that may load at the same time from now on. Fewer: free slots are parked at once, busy ones as their
    /// page finishes. More: parked slots are released at once.
    pub fn set_parallel(&self, n: usize) {
        use std::sync::atomic::Ordering;
        let want = PARALLEL_MAX - n.clamp(PARALLEL_MIN, PARALLEL_MAX);
        self.want_parked.store(want, Ordering::SeqCst);
        let mut parked = lock(&self.parked);
        parked.truncate(want);
        while parked.len() < want {
            match self.slots.clone().try_acquire_owned() {
                Ok(p) => parked.push(p),
                Err(_) => {
                    let (slots, parked, target) = (self.slots.clone(), self.parked.clone(), self.want_parked.clone());
                    tokio::spawn(async move {
                        while let Ok(p) = slots.clone().acquire_owned().await {
                            let mut list = lock(&parked);
                            if list.len() >= target.load(Ordering::SeqCst) {
                                break;
                            }
                            list.push(p);
                        }
                    });
                    break;
                }
            }
        }
    }

    /// Pages that may load at the same time right now.
    pub fn parallel(&self) -> usize {
        PARALLEL_MAX - lock(&self.parked).len()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `2026-11-20T18:04:05Z` from Unix milliseconds.
pub(crate) fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// How a provider is treated in a full scan, in queue order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Plan {
    /// Had a stream in its last scan: first in line, and tried a second time if it has none now.
    Proven,
    /// Never scanned.
    Unknown,
    /// No stream in its last scan(s), but not often enough to skip it.
    Failing,
    /// Would be skipped, but its last try is a day old: tried again at the end.
    Retest,
    /// No stream in the last `SKIP_AFTER` scans: not opened, reported with its last outcome.
    Skip,
}

fn plan(h: Option<&Health>, now: i64) -> Plan {
    match h {
        None => Plan::Unknown,
        Some(h) if h.fail_streak == 0 => Plan::Proven,
        Some(h) if h.fail_streak < SKIP_AFTER => Plan::Failing,
        Some(h) if now - h.last_try >= RETEST_MS => Plan::Retest,
        Some(_) => Plan::Skip,
    }
}

/// Indexes into `ids` in the order the providers are scanned: by plan, proven ones fastest first,
/// otherwise in list order (`providers.json` is best first).
fn queue(ids: &[&str], health: &HashMap<String, Health>, now: i64) -> Vec<(usize, Plan)> {
    let mut q: Vec<(usize, Plan, i64)> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let h = health.get(*id);
            let p = plan(h, now);
            let speed = if p == Plan::Proven { h.and_then(|h| h.avg_ms).unwrap_or(0) } else { 0 };
            (i, p, speed)
        })
        .collect();
    q.sort_by_key(|&(i, p, speed)| (p, speed, i));
    q.into_iter().map(|(i, p, _)| (i, p)).collect()
}

fn skipped(src: &Source, h: Option<&Health>) -> ScanResult {
    let mut r = ScanResult::pending(src);
    let (is_error, streak) = h.map_or((false, SKIP_AFTER), |h| (h.last_error, h.fail_streak));
    let why = match h.and_then(|h| h.last_reason.as_deref()) {
        Some(reason) if is_error => reason,
        _ if is_error => "not reachable",
        _ => "no stream",
    };
    r.status = if is_error { Status::Error } else { Status::NoStream };
    r.error = Some(format!("skipped: {why} in the last {streak} scans (tried again once a day)"));
    r
}

struct Job {
    slot: usize,
    src: Source,
    plan: Plan,
}

type St = State<Arc<AppState>>;
type Params = Query<HashMap<String, String>>;

pub async fn caps(State(st): St) -> Json<Value> {
    Json(json!({ "sniffer": st.sniffer.is_some() }))
}

/// The running scan, or the last finished one if it is younger than 6 h.
fn current(st: &AppState, key: &str) -> ApiResult<Option<Shared>> {
    let fresh = |d: &ScanData| d.running || now_ms() - d.finished_at.unwrap_or(0) < REUSE_MS;
    let cached = lock(&st.scans.map).get(key).cloned();
    if let Some(scan) = cached {
        let is_fresh = fresh(&lock(&scan));
        return Ok(is_fresh.then_some(scan));
    }
    let stored = st.db.blob_get(Table::Scans, key)?.and_then(|d| serde_json::from_str::<ScanData>(&d).ok());
    Ok(stored.filter(fresh).map(|d| Arc::new(Mutex::new(d))))
}

fn idle(st: &AppState, existing: Option<Shared>) -> Json<Value> {
    let mut body = match existing {
        Some(scan) => lock(&scan).body(),
        None => json!({ "state": "idle", "started_at": null, "finished_at": null, "results": [] }),
    };
    if st.sniffer.is_none() {
        body["state"] = json!("unavailable");
    }
    Json(body)
}

pub async fn get(State(st): St, Path((ty, id)): Path<(String, String)>, Query(q): Params) -> ApiResult<Json<Value>> {
    let t = api::title_ref(&ty, &id, &q)?;
    Ok(idle(&st, current(&st, &t.key())?))
}

pub async fn start(
    State(st): St,
    Path((ty, id)): Path<(String, String)>,
    Query(q): Params,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let t = api::title_ref(&ty, &id, &q)?;
    let key = t.key();
    let existing = current(&st, &key)?;
    let Some(sniffer) = st.sniffer.clone() else {
        return Ok((StatusCode::OK, idle(&st, existing)));
    };
    let force = q.get("force").is_some_and(|v| v == "1");
    let only = q.get("provider").filter(|_| force);
    if let Some(scan) = &existing {
        let d = lock(scan);
        if d.running || !force {
            return Ok((StatusCode::ACCEPTED, Json(d.body())));
        }
    }

    let sources = api::build_sources(&st, &t).await?;
    let health_prefix = format!("{}:", t.kind.as_str());
    // `provider=<id>` re-scans one provider and keeps the other results.
    let mut data = match (&existing, only) {
        (Some(old), Some(_)) => lock(old).clone(),
        _ => ScanData { running: false, started_at: now_ms(), finished_at: None, results: Vec::new() },
    };
    let mut jobs = Vec::new();
    let mut replaced = Vec::new();
    match only {
        Some(pid) if existing.is_some() => {
            let src = sources
                .into_iter()
                .find(|s| &s.id == pid)
                .ok_or_else(|| ApiError::bad_request("unknown provider"))?;
            let slot = match data.results.iter().position(|r| &r.provider_id == pid) {
                Some(i) => i,
                None => {
                    data.results.push(ScanResult::pending(&src));
                    data.results.len() - 1
                }
            };
            replaced.extend(data.results[slot].stream_id.take());
            data.results[slot] = ScanResult::pending(&src);
            // asked for by name: never skipped
            jobs.push(Job { slot, src, plan: Plan::Unknown });
        }
        _ => {
            if let Some(old) = &existing {
                replaced.extend(lock(old).results.iter().filter_map(|r| r.stream_id.clone()));
            }
            data.results = sources.iter().map(ScanResult::pending).collect();
            let health = st.db.health_all(&health_prefix)?;
            let ids: Vec<&str> = sources.iter().map(|s| s.id.as_str()).collect();
            let order = queue(&ids, &health, now_ms());
            let mut sources: Vec<Option<Source>> = sources.into_iter().map(Some).collect();
            for (slot, plan) in order {
                let Some(src) = sources[slot].take() else { continue };
                if plan == Plan::Skip {
                    data.results[slot] = skipped(&src, health.get(&src.id));
                } else {
                    jobs.push(Job { slot, src, plan });
                }
            }
        }
    }
    data.running = true;
    data.finished_at = None;
    let body = data.body();
    let scan = Arc::new(Mutex::new(data));
    lock(&st.scans.map).insert(key.clone(), scan.clone());
    tokio::spawn(run(st.clone(), sniffer, key, scan, jobs, replaced, health_prefix));
    Ok((StatusCode::ACCEPTED, Json(body)))
}

async fn run(
    st: Arc<AppState>,
    sniffer: Arc<dyn Sniffer>,
    key: String,
    scan: Shared,
    jobs: Vec<Job>,
    replaced: Vec<String>,
    health_prefix: String,
) {
    for id in replaced {
        stream::forget(&st, &id).await;
    }
    let tried: Vec<(usize, String)> = jobs.iter().map(|j| (j.slot, j.src.id.clone())).collect();

    let mut tasks = tokio::task::JoinSet::new();
    for job in jobs {
        let Ok(permit) = st.scans.slots.clone().acquire_owned().await else { break };
        let (st, sniffer, scan) = (st.clone(), sniffer.clone(), scan.clone());
        tasks.spawn(async move {
            let _permit = permit;
            lock(&scan).results[job.slot].status = Status::Running;
            let mut result = ScanResult::pending(&job.src);
            probe(&st, sniffer.as_ref(), &job.src, &mut result).await;
            if job.plan == Plan::Proven && result.status == Status::NoStream {
                // It had a stream last time, and pages do fail to get going now and then (they
                // nearly always work when loaded on their own): one more try.
                result = ScanResult::pending(&job.src);
                probe(&st, sniffer.as_ref(), &job.src, &mut result).await;
            }
            lock(&scan).results[job.slot] = result;
        });
        // The sniffer starts its pages one after the other; give this one time to get in line so
        // that they start in queue order.
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    while tasks.join_next().await.is_some() {}
    let mut outcomes: Vec<(String, Outcome)> = {
        let mut d = lock(&scan);
        // a provider task that died must not stay "running" forever
        for r in d.results.iter_mut().filter(|r| matches!(r.status, Status::Pending | Status::Running)) {
            r.status = Status::Error;
            r.error = Some("scan task failed".into());
        }
        tried
            .into_iter()
            .map(|(slot, id)| {
                let r = &d.results[slot];
                let outcome = match r.status {
                    Status::Ok => Ok(r.ms.unwrap_or(0)),
                    _ => Err((r.status == Status::Error, r.error.clone())),
                };
                (format!("{health_prefix}{id}"), outcome)
            })
            .collect()
    };
    // Failures only count when something else worked in the same scan: with the network down (or a
    // single provider re-scanned) "no stream" says nothing about the provider.
    let any_ok = outcomes.len() > 1 && outcomes.iter().any(|(_, o)| o.is_ok());
    outcomes.retain(|(_, o)| o.is_ok() || any_ok);
    // before the scan reports "done": the next scan must already see these
    let _ = st.db.health_record(&outcomes);
    let stored = {
        let mut d = lock(&scan);
        d.running = false;
        d.finished_at = Some(now_ms());
        serde_json::to_string(&*d).unwrap_or_default()
    };
    let _ = st.db.blob_put(Table::Scans, &key, &stored);
}

struct Found {
    kind: StreamKind,
    candidate: Candidate,
    ms: u64,
    variants: Vec<Variant>,
    audio: Vec<Track>,
    subtitles: Vec<Track>,
}

async fn probe(st: &AppState, sniffer: &dyn Sniffer, src: &Source, out: &mut ScanResult) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let t0 = Instant::now();
    let mut task = Some(tokio::spawn(sniffer.sniff(src.url.clone(), tx)));
    let mut seen = HashSet::new();
    let mut found: Option<Found> = None;
    let mut settle_at: Option<Instant> = None;
    let mut hint: Option<String> = None;

    loop {
        let until = settle_at.unwrap_or(t0 + PROBE_LIMIT).min(t0 + PROBE_LIMIT);
        let Ok(next) = tokio::time::timeout_at(until.into(), rx.recv()).await else { break };
        let Some(c) = next else {
            // the sniffer ended on its own: page failure or its time is up
            if found.is_none()
                && let Some(t) = task.take()
                && let Ok(Err(e)) = t.await
            {
                out.status = Status::Error;
                out.error = Some(e);
                return;
            }
            break;
        };
        if !seen.insert(c.url.clone()) {
            continue;
        }
        let ms = t0.elapsed().as_millis() as u64;
        match c.kind {
            CandidateKind::Hls => match check_hls(st, &c).await {
                Ok((variants, audio, subtitles)) => {
                    let master = variants.iter().any(|v| v.bandwidth.is_some() || v.height.is_some());
                    // A bare media playlist beats a plain file, but a master may still follow it.
                    if master || !matches!(&found, Some(f) if f.kind == StreamKind::Hls) {
                        found = Some(Found { kind: StreamKind::Hls, candidate: c, ms, variants, audio, subtitles });
                    }
                    if master {
                        break;
                    }
                    settle_at.get_or_insert(Instant::now() + MP4_GRACE);
                }
                Err(e) => hint = Some(e),
            },
            CandidateKind::Mp4 if found.is_none() => match check_mp4(st, &c).await {
                Ok(()) => {
                    let single = Variant { width: None, height: None, bandwidth: None, codecs: None };
                    found = Some(Found {
                        kind: StreamKind::Mp4,
                        candidate: c,
                        ms,
                        variants: vec![single],
                        audio: Vec::new(),
                        subtitles: Vec::new(),
                    });
                    settle_at = Some(Instant::now() + MP4_GRACE);
                }
                Err(e) => hint = hint.or(Some(e)),
            },
            CandidateKind::Mp4 => {}
            CandidateKind::Dash => {
                hint = Some("provider only offers a DASH stream (not supported)".into());
                settle_at.get_or_insert(Instant::now() + MP4_GRACE);
            }
        }
    }
    drop(rx); // tells the sniffer to close its page

    let Some(f) = found else {
        out.status = Status::NoStream;
        out.error = hint;
        return;
    };
    let row = StreamRow {
        id: new_id(&src.url),
        url: f.candidate.url,
        headers: kept(&f.candidate.headers),
        kind: f.kind,
    };
    let saved = serde_json::to_string(&row)
        .map_err(|_| ApiError::internal("could not encode stream"))
        .and_then(|data| st.db.blob_put(Table::Streams, &row.id, &data));
    if saved.is_err() {
        out.status = Status::Error;
        out.error = Some("could not store the stream".into());
        return;
    }
    out.status = Status::Ok;
    out.ms = Some(f.ms);
    out.max_width = f.variants.first().and_then(|v| v.width);
    out.max_height = f.variants.first().and_then(|v| v.height);
    out.max_bandwidth = f.variants.iter().filter_map(|v| v.bandwidth).max();
    out.variants = f.variants;
    out.audio = f.audio;
    out.subtitles = f.subtitles;
    out.stream_id = Some(row.id);
    out.kind = Some(f.kind);
}

fn kept(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers.iter().filter(|(k, _)| KEEP_HEADERS.contains(&k.to_ascii_lowercase().as_str())).cloned().collect()
}

/// Fetches the candidate the way the proxy will and reads its tracks.
async fn check_hls(st: &AppState, c: &Candidate) -> Result<(Vec<Variant>, Vec<Track>, Vec<Track>), String> {
    let resp = st
        .streams
        .request(&kept(&c.headers), &c.url, None)
        .timeout(Duration::from_secs(12))
        .send()
        .await
        .map_err(|_| "playlist not reachable from the server".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("playlist answered HTTP {} to the server", resp.status().as_u16()));
    }
    let body = resp.text().await.map_err(|_| "playlist transfer failed".to_string())?;
    if !hls::is_playlist(&body) {
        return Err("captured URL is not a playlist".into());
    }
    match hls::parse_master(&body) {
        Some(m) => Ok((m.variants.into_iter().map(|v| v.info).collect(), m.audio, m.subtitles)),
        None if !hls::parse_media(&body).is_empty() => {
            Ok((vec![Variant { width: None, height: None, bandwidth: None, codecs: None }], Vec::new(), Vec::new()))
        }
        None => Err("playlist has no variants or segments".into()),
    }
}

async fn check_mp4(st: &AppState, c: &Candidate) -> Result<(), String> {
    let resp = st
        .streams
        .request(&kept(&c.headers), &c.url, Some("bytes=0-0"))
        .timeout(Duration::from_secs(12))
        .send()
        .await
        .map_err(|_| "file not reachable from the server".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("file answered HTTP {} to the server", resp.status().as_u16()));
    }
    let header = |n: &str| resp.headers().get(n).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let total = header("content-range")
        .rsplit('/')
        .next()
        .and_then(|t| t.parse::<u64>().ok())
        .or_else(|| header("content-length").parse().ok())
        .unwrap_or(0);
    if !header("content-type").starts_with("video/") || total < MP4_MIN_BYTES {
        return Err("mp4 request is not a full video file".into());
    }
    Ok(())
}

fn new_id(seed: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut h);
    std::time::SystemTime::now().hash(&mut h);
    COUNTER.fetch_add(1, Ordering::Relaxed).hash(&mut h);
    format!("{:016x}", h.finish())
}

// ---------- settings ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Filter {
    pub min_height: u32,
    pub audio_lang: String,
    pub sort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub filter: Filter,
    pub prefetch_minutes: u32,
    /// Provider pages loading at the same time during a scan (4..=12).
    #[serde(default = "default_parallel")]
    pub parallel: u32,
}

fn default_parallel() -> u32 {
    PARALLEL as u32
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            filter: Filter { min_height: 0, audio_lang: "any".into(), sort: "quality".into() },
            prefetch_minutes: 10,
            parallel: default_parallel(),
        }
    }
}

const SETTINGS_KEY: &str = "settings";

pub fn load_settings(st: &AppState) -> ApiResult<Settings> {
    Ok(st
        .db
        .blob_get(Table::Settings, SETTINGS_KEY)?
        .and_then(|d| serde_json::from_str(&d).ok())
        .unwrap_or_default())
}

pub async fn settings_get(State(st): St) -> ApiResult<Json<Settings>> {
    load_settings(&st).map(Json)
}

pub async fn settings_put(State(st): St, body: Bytes) -> ApiResult<StatusCode> {
    let s: Settings =
        serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(format!("invalid Settings body: {e}")))?;
    if ![0, 480, 720, 1080, 2160].contains(&s.filter.min_height) {
        return Err(ApiError::bad_request("filter.min_height must be 0, 480, 720, 1080 or 2160"));
    }
    if !["any", "de", "en"].contains(&s.filter.audio_lang.as_str()) {
        return Err(ApiError::bad_request("filter.audio_lang must be 'any', 'de' or 'en'"));
    }
    if !["quality", "bitrate", "speed"].contains(&s.filter.sort.as_str()) {
        return Err(ApiError::bad_request("filter.sort must be 'quality', 'bitrate' or 'speed'"));
    }
    if !(1..=120).contains(&s.prefetch_minutes) {
        return Err(ApiError::bad_request("prefetch_minutes must be between 1 and 120"));
    }
    if !(PARALLEL_MIN as u32..=PARALLEL_MAX as u32).contains(&s.parallel) {
        return Err(ApiError::bad_request("parallel must be between 4 and 12"));
    }
    let data = serde_json::to_string(&s).map_err(|_| ApiError::internal("could not encode settings"))?;
    st.db.blob_put(Table::Settings, SETTINGS_KEY, &data)?;
    st.scans.set_parallel(s.parallel as usize);
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{Plan, RETEST_MS, SKIP_AFTER, Status, plan, queue, skipped};
    use crate::{api::Source, db::Health};

    const NOW: i64 = 10 * RETEST_MS;

    fn ok(avg_ms: i64) -> Health {
        Health { avg_ms: Some(avg_ms), last_try: NOW - 1000, ..Health::default() }
    }

    fn failed(streak: u32, age_ms: i64) -> Health {
        Health { fail_streak: streak, last_try: NOW - age_ms, ..Health::default() }
    }

    #[test]
    fn plan_by_fail_streak_and_age() {
        assert_eq!(plan(None, NOW), Plan::Unknown);
        assert_eq!(plan(Some(&ok(900)), NOW), Plan::Proven);
        assert_eq!(plan(Some(&failed(1, 1000)), NOW), Plan::Failing);
        assert_eq!(plan(Some(&failed(SKIP_AFTER - 1, 5 * RETEST_MS)), NOW), Plan::Failing);
        assert_eq!(plan(Some(&failed(SKIP_AFTER, 1000)), NOW), Plan::Skip);
        assert_eq!(plan(Some(&failed(SKIP_AFTER + 4, RETEST_MS - 1)), NOW), Plan::Skip);
        assert_eq!(plan(Some(&failed(SKIP_AFTER, RETEST_MS)), NOW), Plan::Retest);
    }

    #[test]
    fn queue_order() {
        let ids = ["new_a", "slow", "dead", "failing", "fast", "new_b", "retest"];
        let health = HashMap::from([
            ("slow".to_string(), ok(9000)),
            ("fast".to_string(), ok(400)),
            ("dead".to_string(), failed(SKIP_AFTER, 1000)),
            ("failing".to_string(), failed(1, 1000)),
            ("retest".to_string(), failed(SKIP_AFTER, 2 * RETEST_MS)),
        ]);
        let order: Vec<(&str, Plan)> = queue(&ids, &health, NOW).into_iter().map(|(i, p)| (ids[i], p)).collect();
        assert_eq!(
            order,
            [
                ("fast", Plan::Proven),
                ("slow", Plan::Proven),
                ("new_a", Plan::Unknown),
                ("new_b", Plan::Unknown),
                ("failing", Plan::Failing),
                ("retest", Plan::Retest),
                ("dead", Plan::Skip),
            ]
        );
    }

    #[test]
    fn queue_without_health_keeps_list_order() {
        let ids = ["a", "b", "c"];
        let order: Vec<usize> = queue(&ids, &HashMap::new(), NOW).into_iter().map(|(i, _)| i).collect();
        assert_eq!(order, [0, 1, 2]);
    }

    #[test]
    fn skipped_result_keeps_the_last_outcome() {
        let src = Source { id: "x".into(), name: "X".into(), no_ads: false, url: "https://x.example/1".into() };
        let r = skipped(&src, Some(&failed(SKIP_AFTER, 1000)));
        assert_eq!(r.status, Status::NoStream);
        assert_eq!(r.error.as_deref(), Some("skipped: no stream in the last 3 scans (tried again once a day)"));

        let offline = Health { last_error: true, last_reason: Some("provider host not found".into()), ..failed(5, 1000) };
        let r = skipped(&src, Some(&offline));
        assert_eq!(r.status, Status::Error);
        assert_eq!(
            r.error.as_deref(),
            Some("skipped: provider host not found in the last 5 scans (tried again once a day)")
        );
        assert!(r.stream_id.is_none());
    }

    #[test]
    fn iso_dates() {
        assert_eq!(super::iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::iso(1_795_197_845_000), "2026-11-20T18:04:05Z");
        assert_eq!(super::iso(951_782_400_000), "2000-02-29T00:00:00Z");
    }

    #[tokio::test]
    async fn parallel_setting_resizes_without_cutting_running_pages() {
        let s = super::Scans::default();
        assert_eq!(s.parallel(), 10);
        s.set_parallel(12);
        assert_eq!(s.parallel(), 12);
        s.set_parallel(1);
        assert_eq!(s.parallel(), 4, "clamped to the minimum");
        // Four pages busy, then down to 4 is already true; up to 6 frees two slots at once.
        let busy: Vec<_> = (0..4).map(|_| s.slots.clone().try_acquire_owned().unwrap()).collect();
        s.set_parallel(6);
        assert_eq!(s.slots.available_permits(), 2);
        // Down to 4 while six are busy: the slots park as soon as pages finish.
        let more: Vec<_> = (0..2).map(|_| s.slots.clone().try_acquire_owned().unwrap()).collect();
        s.set_parallel(4);
        drop(more);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(s.parallel(), 4);
        assert_eq!(s.slots.available_permits(), 0);
        drop(busy);
    }
}
