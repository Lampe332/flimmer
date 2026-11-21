//! Torrent engine (embedded librqbit) behind the VPN. The engine's only network paths are the SOCKS
//! server of [`crate::vpn`] (outgoing peers), a loopback listener fed by the tunnel's forwarded port
//! (incoming peers) and the tracker shim of [`crate::tracker`]. Its own DHT, local discovery, UPnP and
//! tracker traffic are off: trackers and the DHT are asked by Flimmer through the tunnel.
//! Seeding stops when a download is complete.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::{Ipv4Addr, SocketAddr},
    num::NonZeroU32,
    path::{Component, Path as FsPath, PathBuf},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
};
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, Api, ConnectionOptions, ListenerMode, ListenerOptions, ManagedTorrent,
    PeerConnectionOptions, Session, SessionOptions, TorrentStatsState, limits::LimitsConfig,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::{
    api::{AppState, Params, St},
    db::{Table, now_ms},
    error::{ApiError, ApiResult},
    release::{self, clean_hash, parse_magnet},
    scan::iso,
    tracker::{self, SHIM_INTERVAL, Source, Swarm},
    tsearch::Searcher,
    vpn::{Route, SHIM_HOST, Vpn},
};

const SETTINGS_KEY: &str = "torrents";
const METADATA_TIMEOUT: Duration = Duration::from_secs(120);
const RETRY_EVERY: Duration = Duration::from_secs(15);
/// The tunnel closes this long after the last torrent stopped needing it.
const TUNNEL_LINGER: Duration = Duration::from_secs(5);
const VIDEO_EXT: &[&str] = &["mkv", "mp4", "m4v", "avi", "mov", "webm", "ts", "m2ts", "wmv", "mpg", "mpeg"];
/// Events kept for the UI (finished / failed / grabbed).
const EVENTS_MAX: usize = 200;
const GIB: u64 = 1024 * 1024 * 1024;
const PEER_LIMIT: usize = 150;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentSettings {
    /// WireGuard config of the VPN (read on every tunnel start, never copied).
    pub vpn_config: String,
    pub movies_dir: String,
    pub series_dir: String,
    /// Upload limit while downloading, KiB/s (1..=1000).
    pub upload_kbps: u32,
    /// Downloads running at once (1..=10); the rest waits in the queue.
    #[serde(default = "default_max_active")]
    pub max_active: u32,
    /// Free space a download must leave on its drive, GiB.
    #[serde(default = "default_min_free_gb")]
    pub min_free_gb: u32,
    /// Search sources turned off (ids of `GET /api/torrents/sources`).
    #[serde(default = "crate::tsearch::default_off")]
    pub sources_off: Vec<String>,
    /// Torrents may run without VPN (the user confirmed the warning). Off by default.
    #[serde(default)]
    pub allow_direct: bool,
    /// Without VPN even when a VPN file is there (only with `allow_direct`).
    #[serde(default)]
    pub direct: bool,
}

fn default_max_active() -> u32 {
    3
}

fn default_min_free_gb() -> u32 {
    5
}

impl TorrentSettings {
    fn defaults(root: &FsPath) -> Self {
        let dir = |name: &str, fallback: &str| {
            crate::host::get().map_or_else(|| fallback.to_owned(), |h| h.media_dir.join(name).to_string_lossy().into_owned())
        };
        Self {
            vpn_config: root.join("data").join("vpn").join("proton.conf").to_string_lossy().into_owned(),
            movies_dir: dir("Filme", r"D:\Flimmer Downloads\Filme"),
            series_dir: dir("Serien", r"D:\Flimmer Downloads\Serien"),
            upload_kbps: 30,
            max_active: default_max_active(),
            min_free_gb: default_min_free_gb(),
            sources_off: crate::tsearch::default_off(),
            allow_direct: false,
            direct: false,
        }
    }
}

/// What a torrent is, as far as the library is concerned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Media {
    /// "movie" | "tv"
    #[serde(rename = "type")]
    pub kind: String,
    /// TMDB id.
    pub id: Option<u64>,
    pub title: String,
    pub year: Option<u32>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMeta {
    pub index: usize,
    /// Path inside the torrent.
    pub path: String,
    pub size: u64,
}

/// Persisted state of one torrent (SQLite table `torrents`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    pub name: String,
    /// "stream" | "download"
    pub mode: String,
    pub media: Option<Media>,
    pub trackers: Vec<String>,
    pub folder: String,
    pub files: Vec<FileMeta>,
    pub selected: Vec<usize>,
    /// Files asked for before the metadata was known.
    pub wanted: Option<Vec<usize>>,
    pub paused: bool,
    pub completed: bool,
    pub added_at: i64,
    /// Queue priority: higher starts first (-100..=100).
    #[serde(default)]
    pub priority: i32,
    /// Queued by an auto-download rule.
    #[serde(default)]
    pub auto: bool,
}

/// Something the UI should tell the user about (`GET /api/torrents/events`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: u64,
    pub at: String,
    /// "finished" | "failed" | "grabbed"
    pub kind: String,
    pub torrent_id: String,
    pub name: String,
    pub media: Option<Media>,
    pub message: Option<String>,
}

pub struct Entry {
    pub id: String,
    pub meta: Mutex<Meta>,
    pub handle: Mutex<Option<Arc<ManagedTorrent>>>,
    pub swarm: Arc<Swarm>,
    announcer: Mutex<Option<CancellationToken>>,
    error: Mutex<Option<String>>,
    busy: AtomicBool,
    last_try: Mutex<Option<Instant>>,
    /// Waiting for a free download slot.
    queued: AtomicBool,
    /// The failure the user was last told about (one event per distinct error).
    told: Mutex<Option<String>>,
}

impl Entry {
    fn new(meta: Meta) -> Arc<Self> {
        Arc::new(Self {
            id: meta.id.clone(),
            meta: Mutex::new(meta),
            handle: Mutex::new(None),
            swarm: Arc::default(),
            announcer: Mutex::new(None),
            error: Mutex::new(None),
            busy: AtomicBool::new(false),
            last_try: Mutex::new(None),
            queued: AtomicBool::new(false),
            told: Mutex::new(None),
        })
    }

    pub fn meta(&self) -> Meta {
        self.meta.lock().unwrap().clone()
    }

    pub fn handle(&self) -> Option<Arc<ManagedTorrent>> {
        self.handle.lock().unwrap().clone()
    }

    fn stop_announcer(&self) {
        if let Some(token) = self.announcer.lock().unwrap().take() {
            token.cancel();
        }
    }
}

pub struct Torrents {
    pub vpn: Arc<Vpn>,
    pub searcher: Searcher,
    pub root: PathBuf,
    pub port: u16,
    session: Arc<Session>,
    api: Api,
    entries: Mutex<HashMap<String, Arc<Entry>>>,
    peer_id: [u8; 20],
    pub(crate) probes: Mutex<HashMap<(String, usize), Arc<crate::play::Probe>>>,
    /// Files being read right now (player or ffmpeg), for the readahead window and `stream_health`.
    pub(crate) playing: Mutex<HashMap<(String, usize), Arc<crate::play::Playback>>>,
    events: Mutex<VecDeque<Event>>,
    next_event: AtomicU64,
}

// ---------- pure helpers ----------

/// A string that is safe as one Windows path component.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c })
        .collect();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = joined.trim_matches(|c| c == '.' || c == ' ');
    let stem = trimmed.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    match (trimmed.is_empty(), reserved) {
        (true, _) => "Unbenannt".into(),
        (_, true) => format!("_{trimmed}"),
        _ => trimmed.chars().take(120).collect::<String>().trim_end_matches(['.', ' ']).to_owned(),
    }
}

/// `Filme\<Titel (Jahr)>` or `Serien\<Titel>\Staffel NN`; without media info `Filme\<torrent name>`.
pub fn folder_for(settings: &TorrentSettings, media: Option<&Media>, torrent_name: &str) -> PathBuf {
    match media {
        Some(m) if m.kind == "tv" => {
            let series = PathBuf::from(&settings.series_dir).join(sanitize(&m.title));
            match m.season {
                Some(s) => series.join(format!("Staffel {s:02}")),
                None => series,
            }
        }
        Some(m) => PathBuf::from(&settings.movies_dir).join(sanitize(&match m.year {
            Some(y) => format!("{} ({y})", m.title),
            None => m.title.clone(),
        })),
        None => PathBuf::from(&settings.movies_dir).join(sanitize(torrent_name)),
    }
}

pub fn is_video(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit(['/', '\\']).next().unwrap_or_default();
    VIDEO_EXT.iter().any(|e| name.ends_with(&format!(".{e}"))) && !name.contains("sample")
}

/// Files to download: the explicit wish, else the requested episode, else (a season) every video,
/// else the largest video.
pub fn choose_files(files: &[FileMeta], wanted: Option<&[usize]>, media: Option<&Media>) -> Vec<usize> {
    if let Some(w) = wanted {
        let valid: Vec<usize> = w.iter().copied().filter(|i| files.iter().any(|f| f.index == *i)).collect();
        if !valid.is_empty() {
            return valid;
        }
    }
    let videos: Vec<&FileMeta> = files.iter().filter(|f| is_video(&f.path)).collect();
    if let Some(m) = media.filter(|m| m.kind == "tv") {
        match m.episode {
            Some(ep) => {
                let hit = videos.iter().find(|f| {
                    let r = release::parse(f.path.rsplit(['/', '\\']).next().unwrap_or_default());
                    r.episode == Some(ep) && (r.season.is_none() || r.season == m.season)
                });
                if let Some(f) = hit {
                    return vec![f.index];
                }
            }
            None if !videos.is_empty() => return videos.iter().map(|f| f.index).collect(),
            None => {}
        }
    }
    let pool = if videos.is_empty() { files.iter().collect() } else { videos };
    pool.into_iter().max_by_key(|f| f.size).map(|f| vec![f.index]).unwrap_or_default()
}

static EP_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:e|ep|episode|folge)[ ._]?(\d{1,3})\b").expect("static regex"));
static EP_DASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[ _\]])-[ _](\d{2,3})(?:v\d)?(?:[ _\[(.]|$)").expect("static regex"));
static SEASON_DIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:season|staffel|s)[ ._]?(\d{1,2})\b").expect("static regex"));

/// (season, episode) of a video file inside a torrent: "S01E02", "1x02", "Folge 2", "Show - 02";
/// the season falls back to the folder ("Season 1", "Staffel 1") and then to `default_season`.
pub fn episode_of(path: &str, default_season: Option<u32>) -> Option<(u32, u32)> {
    let (dir, name) = path.rsplit_once(['/', '\\']).unwrap_or(("", path));
    let r = release::parse(name);
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let episode = r
        .episode
        .or_else(|| EP_WORD.captures(stem).and_then(|c| c[1].parse().ok()))
        .or_else(|| EP_DASH.captures(stem).and_then(|c| c[1].parse().ok()))?;
    let from_dir = || dir.rsplit(['/', '\\']).find_map(|d| SEASON_DIR.captures(d).and_then(|c| c[1].parse().ok()));
    Some((r.season.or_else(from_dir).or(default_season).unwrap_or(1), episode))
}

/// Video files with their episode, in episode order.
pub fn episodes(files: &[FileMeta], default_season: Option<u32>) -> Vec<(u32, u32, usize)> {
    let mut list: Vec<(u32, u32, usize)> = files
        .iter()
        .filter(|f| is_video(&f.path))
        .filter_map(|f| episode_of(&f.path, default_season).map(|(s, e)| (s, e, f.index)))
        .collect();
    list.sort();
    list
}

/// The file of the episode after the one in file `current`.
pub fn next_episode(files: &[FileMeta], default_season: Option<u32>, current: usize) -> Option<(u32, u32, usize)> {
    let list = episodes(files, default_season);
    let here = list.iter().find(|(_, _, i)| *i == current).map(|(s, e, _)| (*s, *e))?;
    list.into_iter().find(|(s, e, _)| (*s, *e) > here)
}

/// Free bytes on the drive that holds `path` (the nearest existing folder above it).
#[cfg(windows)]
pub fn free_space(path: &FsPath) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(dir: *const u16, available: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    }
    let existing = path.ancestors().find(|p| p.is_dir())?;
    let wide: Vec<u16> = existing.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0u64;
    // SAFETY: `wide` is a NUL-terminated UTF-16 string that lives across the call, `available` is a
    // valid `u64` to write to, and the API allows null for the two outputs that are not needed.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(available)
}

#[cfg(not(windows))]
pub fn free_space(_path: &FsPath) -> Option<u64> {
    None
}

/// `Err` with the message for the user when `need` more bytes do not fit while keeping `reserve` free.
pub fn check_space(free: Option<u64>, need: u64, reserve: u64, folder: &FsPath) -> Result<(), String> {
    let Some(free) = free else { return Ok(()) };
    if free >= need.saturating_add(reserve) {
        return Ok(());
    }
    let gb = |b: u64| b as f64 / GIB as f64;
    let drive = folder.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
    Err(format!(
        "not enough disk space on {drive}: {:.1} GB needed, {:.1} GB free ({:.0} GB are kept free)",
        gb(need),
        gb(free),
        gb(reserve)
    ))
}

/// Whether a torrent file names no tracker besides Flimmer's shim (`announce` and `announce-list`).
pub fn only_shim_trackers(torrent: &[u8]) -> bool {
    let Some(meta) = crate::benc::decode(torrent) else { return false };
    let shim = |url: &crate::benc::Benc| url.bytes().and_then(|b| std::str::from_utf8(b).ok()).is_some_and(|u| u == shim_url());
    let single = meta.get("announce").is_none_or(shim);
    let list = meta.get("announce-list").is_none_or(|tiers| tiers.list().iter().all(|tier| tier.list().iter().all(shim)));
    // Web seeds would be fetched over the normal network as well.
    single && list && meta.get("url-list").is_none() && meta.get("httpseeds").is_none() && meta.get("info").is_some()
}

fn shim_url() -> String {
    format!("http://{SHIM_HOST}/api/torrents/announce")
}

fn hash_bytes(id: &str) -> [u8; 20] {
    let mut out = [0u8; 20];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&id[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    out
}

// ---------- engine ----------

impl Torrents {
    pub async fn new(root: PathBuf, port: u16, settings: &TorrentSettings) -> Result<Arc<Self>, String> {
        let vpn = Vpn::new(PathBuf::from(&settings.vpn_config), port, root.join("data").join("cache")).await?;
        vpn.set_direct(settings.allow_direct, settings.direct);
        let data = root.join("data").join("torrents");
        std::fs::create_dir_all(&data).map_err(|e| format!("cannot create {}: {e}", data.display()))?;
        let opts = SessionOptions {
            dht: None,
            // Loopback only: incoming peers reach it through the tunnel's forwarded port (see `vpn`).
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpOnly,
                listen_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                enable_upnp_port_forwarding: false,
                ipv4_only: true,
                ..ListenerOptions::default()
            }),
            // Streams hold a blocking permit each (player, ffmpeg, readahead helpers).
            runtime_worker_threads: Some(32),
            persistence: None,
            fastresume: false,
            disable_local_service_discovery: true,
            ipv4_only: true,
            // Slots per torrent, shared by outgoing attempts and incoming peers. With hundreds of known
            // addresses 50 slots are all busy dialling and an incoming peer never gets one.
            peer_limit: Some(PEER_LIMIT),
            connect: Some(ConnectionOptions {
                proxy_url: Some(vpn.proxy_url()),
                enable_tcp: false,
                peer_opts: Some(PeerConnectionOptions {
                    connect_timeout: Some(Duration::from_secs(10)),
                    ..PeerConnectionOptions::default()
                }),
            }),
            ratelimits: LimitsConfig { upload_bps: upload_limit(settings.upload_kbps), download_bps: None },
            ..SessionOptions::default()
        };
        let session = Session::new_with_opts(data, opts).await.map_err(|e| format!("torrent engine: {e:#}"))?;
        match session.listen_addr() {
            Some(addr) if addr.ip().is_loopback() => vpn.set_engine_port(addr.port()),
            Some(_) => return Err("torrent engine: the peer listener must be loopback only".into()),
            None => {}
        }
        let mut peer_id = *b"-FL0200-000000000000";
        peer_id[8..].copy_from_slice(&rand::random::<[u8; 12]>().map(|b| b'0' + b % 10));
        Ok(Arc::new(Self {
            vpn,
            searcher: Searcher::new()?,
            root,
            port,
            api: Api::new(session.clone(), None),
            session,
            entries: Mutex::default(),
            peer_id,
            probes: Mutex::default(),
            playing: Mutex::default(),
            events: Mutex::default(),
            next_event: AtomicU64::new(1),
        }))
    }

    pub fn entry(&self, id: &str) -> ApiResult<Arc<Entry>> {
        self.entries.lock().unwrap().get(id).cloned().ok_or_else(|| ApiError::not_found("unknown torrent"))
    }

    pub fn swarm(&self, id: &str) -> Option<Arc<Swarm>> {
        self.entries.lock().unwrap().get(id).map(|e| e.swarm.clone())
    }

    fn all(&self) -> Vec<Arc<Entry>> {
        let mut list: Vec<_> = self.entries.lock().unwrap().values().cloned().collect();
        list.sort_by_key(|e| std::cmp::Reverse(e.meta.lock().unwrap().added_at));
        list
    }

    fn torrent_file(&self, id: &str) -> PathBuf {
        self.root.join("data").join("torrents").join(format!("{id}.torrent"))
    }

    fn start_announcer(self: &Arc<Self>, e: &Arc<Entry>) {
        let mut slot = e.announcer.lock().unwrap();
        if slot.is_some() {
            return;
        }
        let token = CancellationToken::new();
        *slot = Some(token.clone());
        let trackers = e.meta.lock().unwrap().trackers.clone();
        let hash = hash_bytes(&e.id);
        tokio::spawn(tracker::run(self.vpn.clone(), hash, self.peer_id, trackers, e.swarm.clone(), token.clone()));
        tokio::spawn(crate::dht::run(self.vpn.clone(), hash, e.swarm.clone(), token));
    }

    /// Which pieces are there: (bitfield, most significant bit first; piece length; (offset, length) of `file`).
    pub(crate) fn haves(&self, handle: &Arc<ManagedTorrent>, file: usize) -> Option<(Vec<u8>, u64, u64, u64)> {
        let (piece, offset, len) = handle
            .with_metadata(|m| m.file_infos.get(file).map(|f| (u64::from(m.lengths().default_piece_length()), f.offset_in_torrent, f.len)))
            .ok()??;
        let (bits, _) = self.api.api_dump_haves(handle.id().into()).ok()?;
        Some((bits.as_raw_slice().to_vec(), piece, offset, len))
    }

    /// Events with an id above `since`, oldest first.
    pub fn events_since(&self, since: u64) -> Vec<Event> {
        self.events.lock().unwrap().iter().filter(|e| e.id > since).cloned().collect()
    }

    /// Entries of one title (TMDB id), newest first.
    pub fn of_title(&self, kind: &str, id: u64) -> Vec<Arc<Entry>> {
        self.all().into_iter().filter(|e| e.meta.lock().unwrap().media.as_ref().is_some_and(|m| m.kind == kind && m.id == Some(id))).collect()
    }

    pub fn has(&self, id: &str) -> bool {
        self.entries.lock().unwrap().contains_key(id)
    }
}

/// Records an event for the UI and keeps the newest [`EVENTS_MAX`].
pub fn emit(st: &AppState, kind: &str, e: &Entry, message: Option<String>) {
    let tor = &st.torrents;
    let m = e.meta();
    let event = Event {
        id: tor.next_event.fetch_add(1, Ordering::SeqCst),
        at: iso(now_ms()),
        kind: kind.into(),
        torrent_id: m.id,
        name: m.name,
        media: m.media,
        message,
    };
    if let Ok(data) = serde_json::to_string(&event) {
        let _ = st.db.blob_put(Table::Events, &format!("{:012}", event.id), &data);
    }
    let mut events = tor.events.lock().unwrap();
    events.push_back(event);
    while events.len() > EVENTS_MAX {
        if let Some(old) = events.pop_front() {
            let _ = st.db.blob_delete(Table::Events, &format!("{:012}", old.id));
        }
    }
}

fn upload_limit(kbps: u32) -> Option<NonZeroU32> {
    NonZeroU32::new(kbps.clamp(1, 1000) * 1024)
}

pub fn load_settings(st: &AppState) -> ApiResult<TorrentSettings> {
    Ok(st
        .db
        .blob_get(Table::Settings, SETTINGS_KEY)?
        .and_then(|d| serde_json::from_str(&d).ok())
        .unwrap_or_else(|| TorrentSettings::defaults(&st.torrents.root)))
}

/// Settings before the app state exists (startup).
pub fn initial_settings(db: &crate::db::Db, root: &FsPath) -> TorrentSettings {
    db.blob_get(Table::Settings, SETTINGS_KEY)
        .ok()
        .flatten()
        .and_then(|d| serde_json::from_str(&d).ok())
        .unwrap_or_else(|| TorrentSettings::defaults(root))
}

fn persist(st: &AppState, e: &Entry) {
    if let Ok(data) = serde_json::to_string(&*e.meta.lock().unwrap()) {
        let _ = st.db.blob_put(Table::Torrents, &e.id, &data);
    }
}

/// Makes a torrent live: tunnel up, metadata (from disk or from peers), files chosen, added to the engine.
async fn bring_up(st: Arc<AppState>, e: Arc<Entry>) {
    if e.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    *e.last_try.lock().unwrap() = Some(Instant::now());
    let result = bring_up_inner(&st, &e).await;
    set_error(&st, &e, result.err());
    e.busy.store(false, Ordering::SeqCst);
}

/// Stores the torrent's error; a new one is told to the user once ("failed" event).
fn set_error(st: &AppState, e: &Entry, error: Option<String>) {
    *e.error.lock().unwrap() = error.clone();
    let Some(error) = error else { return };
    let new = e.told.lock().unwrap().as_deref() != Some(error.as_str());
    if new && st.torrents.has(&e.id) {
        *e.told.lock().unwrap() = Some(error.clone());
        emit(st, "failed", e, Some(error));
    }
}

async fn bring_up_inner(st: &Arc<AppState>, e: &Arc<Entry>) -> Result<(), String> {
    let tor = &st.torrents;
    tor.vpn.ensure_up().await?;
    tor.start_announcer(e);
    let trackers = Some(vec![shim_url()]);

    let path = tor.torrent_file(&e.id);
    // The engine announces to every tracker a torrent file names, over its own sockets. A stored
    // file that names anything but the shim is not used: the magnet path below rebuilds it.
    let known = tokio::fs::read(&path).await.ok().filter(|b| !e.meta.lock().unwrap().files.is_empty() && only_shim_trackers(b));
    let (bytes, seen) = match known {
        Some(bytes) => (Bytes::from(bytes), Vec::new()),
        None => {
            // For a magnet the engine only uses the trackers inside the link (`opts.trackers` is ignored).
            let shim: String = url::form_urlencoded::byte_serialize(shim_url().as_bytes()).collect();
            let magnet = format!("magnet:?xt=urn:btih:{}&tr={shim}", e.id);
            let opts = AddTorrentOptions {
                list_only: true,
                trackers: trackers.clone(),
                force_tracker_interval: Some(SHIM_INTERVAL),
                ..AddTorrentOptions::default()
            };
            let resolved = tokio::time::timeout(METADATA_TIMEOUT, tor.session.add_torrent(AddTorrent::from_url(magnet), Some(opts)))
                .await
                .map_err(|_| "no peers found for this torrent (metadata timeout)".to_owned())?
                .map_err(|err| format!("metadata: {err:#}"))?;
            let AddTorrentResponse::ListOnly(list) = resolved else {
                return Err("torrent is already known to the engine".into());
            };
            let files: Vec<FileMeta> = list
                .info
                .iter_file_details()
                .enumerate()
                .map(|(index, d)| FileMeta { index, path: d.filename.to_pathbuf().to_string_lossy().replace('\\', "/"), size: d.len })
                .collect();
            let name = list.info.name().map(|n| n.to_string());
            tokio::fs::write(&path, &list.torrent_bytes).await.map_err(|err| format!("cannot store torrent file: {err}"))?;
            let mut m = e.meta.lock().unwrap();
            if let Some(n) = name.filter(|n| !n.is_empty()) {
                m.name = n;
            }
            m.files = files;
            (list.torrent_bytes, list.seen_peers)
        }
    };

    let settings = load_settings(st).map_err(|_| "cannot read settings".to_owned())?;
    let (selected, folder) = {
        let mut m = e.meta.lock().unwrap();
        if m.selected.is_empty() {
            m.selected = choose_files(&m.files, m.wanted.as_deref(), m.media.as_ref());
        }
        if m.folder.is_empty() {
            m.folder = folder_for(&settings, m.media.as_ref(), &m.name).to_string_lossy().into_owned();
        }
        (m.selected.clone(), m.folder.clone())
    };
    persist(st, e);
    if selected.is_empty() {
        return Err("torrent contains no files".into());
    }
    // Enough room? What is on disk already (a resumed download) does not count again.
    {
        let m = e.meta();
        let dir = PathBuf::from(&folder);
        let need: u64 = m
            .files
            .iter()
            .filter(|f| selected.contains(&f.index))
            .map(|f| f.size.saturating_sub(std::fs::metadata(dir.join(&f.path)).map_or(0, |d| d.len())))
            .sum();
        check_space(free_space(&dir), need, u64::from(settings.min_free_gb) * GIB, &dir)?;
    }

    let opts = AddTorrentOptions {
        only_files: Some(selected),
        output_folder: Some(folder),
        overwrite: true,
        initial_peers: Some(seen),
        trackers,
        force_tracker_interval: Some(SHIM_INTERVAL),
        ..AddTorrentOptions::default()
    };
    let handle = tor
        .session
        .add_torrent(AddTorrent::from_bytes(bytes), Some(opts))
        .await
        .map_err(|err| format!("cannot start torrent: {err:#}"))?
        .into_handle()
        .ok_or("cannot start torrent")?;
    // Paused or deleted while the metadata was resolving.
    let gone = !tor.entries.lock().unwrap().contains_key(&e.id);
    if gone {
        let _ = tor.session.delete(handle.id().into(), false).await;
        return Ok(());
    }
    if e.meta.lock().unwrap().paused {
        let _ = tor.session.pause(&handle).await;
    }
    *e.handle.lock().unwrap() = Some(handle);
    // Watching right away: the file's first and last pieces (container headers, index) come first.
    let m = e.meta();
    if m.mode == "stream" && !m.paused {
        let video = m.selected.iter().copied().find(|i| m.files.iter().any(|f| f.index == *i && is_video(&f.path)));
        if let Some(file) = video.or(m.selected.first().copied()) {
            crate::play::prime(st, e, file);
        }
    }
    Ok(())
}

/// Who gets a download slot: streams the user is watching always, then by priority, then first come.
pub fn queue_order(list: &mut [(bool, i32, i64, usize)]) {
    list.sort_by_key(|(stream, priority, added, _)| (!*stream, std::cmp::Reverse(*priority), *added));
}

/// Background loop: queue, retries, completion (stops seeding) and the tunnel's lifetime.
pub fn spawn_manager(st: Arc<AppState>) {
    for data in st.db.blob_all(Table::Torrents).unwrap_or_default() {
        if let Ok(meta) = serde_json::from_str::<Meta>(&data) {
            st.torrents.entries.lock().unwrap().insert(meta.id.clone(), Entry::new(meta));
        }
    }
    {
        let stored: Vec<Event> = st.db.blob_all(Table::Events).unwrap_or_default().iter().filter_map(|d| serde_json::from_str(d).ok()).collect();
        st.torrents.next_event.store(stored.iter().map(|e| e.id).max().unwrap_or(0) + 1, Ordering::SeqCst);
        *st.torrents.events.lock().unwrap() = stored.into();
    }
    tokio::spawn(async move {
        let tor = st.torrents.clone();
        let mut idle_since: Option<Instant> = None;
        loop {
            let max_active = load_settings(&st).map_or(default_max_active(), |s| s.max_active).clamp(1, 10) as usize;
            let wanting: Vec<Arc<Entry>> = tor
                .all()
                .into_iter()
                .filter(|e| {
                    let m = e.meta.lock().unwrap();
                    let waits = !m.completed && !m.paused;
                    if !waits {
                        e.queued.store(false, Ordering::SeqCst);
                    }
                    waits
                })
                .collect();
            let mut order: Vec<(bool, i32, i64, usize)> = wanting
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let m = e.meta.lock().unwrap();
                    (m.mode == "stream", m.priority, m.added_at, i)
                })
                .collect();
            queue_order(&mut order);

            let mut active = 0;
            for (slot, (stream, _, _, i)) in order.into_iter().enumerate() {
                let e = &wanting[i];
                if slot >= max_active && !stream {
                    // No free slot: no peers, no announces, nothing resolved.
                    e.queued.store(true, Ordering::SeqCst);
                    e.stop_announcer();
                    if let Some(h) = e.handle().filter(|h| !h.is_paused()) {
                        let _ = tor.session.pause(&h).await;
                    }
                    continue;
                }
                let was_queued = e.queued.swap(false, Ordering::SeqCst);
                match e.handle() {
                    Some(h) => {
                        if was_queued && h.is_paused() && !tor.vpn.is_blocked() {
                            let _ = tor.vpn.ensure_up().await;
                            tor.start_announcer(e);
                            let _ = tor.session.unpause(&h).await;
                        }
                        let stats = h.stats();
                        if stats.finished {
                            // Done: stop all peer traffic for it (no seeding).
                            let _ = tor.session.pause(&h).await;
                            e.stop_announcer();
                            e.meta.lock().unwrap().completed = true;
                            persist(&st, e);
                            emit(&st, "finished", e, None);
                            continue;
                        }
                        if matches!(stats.state, TorrentStatsState::Error) {
                            set_error(&st, e, stats.error);
                        }
                    }
                    None => {
                        let due = e.last_try.lock().unwrap().is_none_or(|t| t.elapsed() > RETRY_EVERY);
                        if due && !e.busy.load(Ordering::SeqCst) && !tor.vpn.is_blocked() {
                            tokio::spawn(bring_up(st.clone(), e.clone()));
                        }
                    }
                }
                active += 1;
            }
            if active > 0 {
                idle_since = None;
                if tor.vpn.current().is_none() && !tor.vpn.is_blocked() {
                    let _ = tor.vpn.ensure_up().await;
                }
            } else if idle_since.get_or_insert_with(Instant::now).elapsed() > TUNNEL_LINGER {
                tor.vpn.down();
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

// ---------- API ----------

fn info(tor: &Torrents, e: &Entry) -> Value {
    let m = e.meta();
    let handle = e.handle();
    let stats = handle.as_ref().map(|h| h.stats());
    let error = e.error.lock().unwrap().clone();
    let busy = e.busy.load(Ordering::SeqCst);
    let queued = e.queued.load(Ordering::SeqCst);
    let state = if m.completed {
        "completed"
    } else if m.paused {
        "paused"
    } else if tor.vpn.is_blocked() {
        "vpn_blocked"
    } else if queued {
        "queued"
    } else {
        match stats.as_ref().map(|s| s.state) {
            Some(TorrentStatsState::Initializing { .. }) => "checking",
            Some(TorrentStatsState::Live) => "downloading",
            Some(TorrentStatsState::Paused) => "paused",
            Some(TorrentStatsState::Error) => "error",
            None if error.is_some() && !busy => "error",
            None if m.files.is_empty() => "metadata",
            None => "starting",
        }
    };
    let file_progress = stats.as_ref().map(|s| s.file_progress.clone()).unwrap_or_default();
    let total: u64 = m.files.iter().filter(|f| m.selected.contains(&f.index)).map(|f| f.size).sum();
    let downloaded: u64 = if m.completed {
        total
    } else {
        m.selected.iter().filter_map(|i| file_progress.get(*i)).sum()
    };
    let live = stats.as_ref().and_then(|s| s.live.as_ref()).filter(|_| state == "downloading");
    let down = live.map_or(0.0, |l| l.download_speed.mbps * 1024.0 * 1024.0);
    let up = live.map_or(0.0, |l| l.upload_speed.mbps * 1024.0 * 1024.0);
    let peers = live
        .and_then(|l| serde_json::to_value(&l.snapshot.peer_stats).ok())
        .and_then(|v| v.get("live").and_then(Value::as_u64))
        .unwrap_or(0);
    let folder = PathBuf::from(&m.folder);
    // Connected / known peers by where their address came from.
    let (mut connected, mut known) = ([0u32; 4], [0u32; 4]);
    let peer_stats = handle.as_ref().and_then(|h| h.live()).filter(|_| state == "downloading").and_then(|live| {
        Some(live.per_peer_stats_snapshot(serde_json::from_str(r#"{"state":"all"}"#).ok()?))
    });
    for (addr, peer) in peer_stats.iter().flat_map(|s| s.peers.iter()) {
        let class = match addr.parse::<SocketAddr>() {
            // Incoming peers arrive through the tunnel's forwarded port on the loopback listener.
            Ok(a) if a.ip().is_loopback() => 3,
            Ok(SocketAddr::V4(a)) => match e.swarm.source_of(&a) {
                Some(Source::Tracker) => 0,
                Some(Source::Dht) => 1,
                None => 2,
            },
            _ => 2,
        };
        known[class] += 1;
        connected[class] += u32::from(peer.state == "live");
    }
    let by_source = |c: [u32; 4]| json!({ "tracker": c[0], "dht": c[1], "pex": c[2], "incoming": c[3] });
    let default_season = m.media.as_ref().and_then(|x| x.season);
    let episode = |f: &FileMeta| is_video(&f.path).then(|| episode_of(&f.path, default_season)).flatten();
    let tv = m.media.as_ref().is_none_or(|x| x.kind == "tv");
    let queue_position = queued.then(|| {
        let mut order: Vec<(bool, i32, i64, usize)> = tor
            .all()
            .iter()
            .enumerate()
            .filter(|(_, x)| x.queued.load(Ordering::SeqCst))
            .map(|(i, x)| {
                let xm = x.meta.lock().unwrap();
                (false, xm.priority, xm.added_at, usize::from(xm.id == m.id) + i * 2)
            })
            .collect();
        queue_order(&mut order);
        order.iter().position(|(_, _, _, tag)| tag % 2 == 1).map_or(1, |p| p + 1)
    });
    json!({
        "id": m.id, "name": m.name, "state": state, "mode": m.mode, "media": m.media,
        "priority": m.priority, "queue_position": queue_position, "auto": m.auto,
        "peers_by_source": by_source(connected), "known_by_source": by_source(known),
        "found_by_source": { "tracker": e.swarm.counts().0, "dht": e.swarm.counts().1 },
        "stream_health": crate::play::stream_health(tor, e, &m, handle.as_ref(), down as u64, peers),
        "progress": if total > 0 { downloaded as f64 / total as f64 } else { 0.0 },
        "downloaded": downloaded, "total": total,
        "down_speed": down as u64, "up_speed": up as u64,
        "peers": peers, "peers_known": e.swarm.len(),
        "eta_seconds": (down > 1.0 && total > downloaded).then(|| ((total - downloaded) as f64 / down) as u64),
        "folder": m.folder, "error": error, "added_at": iso(m.added_at),
        "files": m.files.iter().map(|f| json!({
            "index": f.index, "name": f.path, "size": f.size,
            "path": folder.join(&f.path).to_string_lossy(),
            "selected": m.selected.contains(&f.index),
            "downloaded": if m.completed && m.selected.contains(&f.index) { f.size } else { file_progress.get(f.index).copied().unwrap_or(0) },
            "video": is_video(&f.path),
            "season": episode(f).filter(|_| tv).map(|x| x.0), "episode": episode(f).filter(|_| tv).map(|x| x.1),
        })).collect::<Vec<_>>(),
    })
}

#[derive(Deserialize)]
struct StartReq {
    magnet: Option<String>,
    infohash: Option<String>,
    #[serde(default)]
    trackers: Vec<String>,
    name: Option<String>,
    mode: Option<String>,
    file_index: Option<usize>,
    files: Option<Vec<usize>>,
    media: Option<Media>,
    priority: Option<i32>,
}

/// A new torrent for the engine (also what the auto-download job queues).
pub struct NewTorrent {
    pub id: String,
    pub name: String,
    pub mode: String,
    pub media: Option<Media>,
    pub trackers: Vec<String>,
    pub wanted: Option<Vec<usize>>,
    pub priority: i32,
    pub auto: bool,
}

/// Adds a torrent that is not known yet; the manager starts it when a download slot is free.
pub fn add(st: &Arc<AppState>, new: NewTorrent) -> Arc<Entry> {
    let e = Entry::new(Meta {
        id: new.id.clone(),
        name: new.name,
        mode: new.mode,
        media: new.media,
        trackers: new.trackers,
        folder: String::new(),
        files: Vec::new(),
        selected: Vec::new(),
        wanted: new.wanted,
        paused: false,
        completed: false,
        added_at: now_ms(),
        priority: new.priority.clamp(-100, 100),
        auto: new.auto,
    });
    st.torrents.entries.lock().unwrap().insert(new.id, e.clone());
    persist(st, &e);
    e
}

/// `POST /api/torrents/start`
pub async fn start(State(st): St, body: Bytes) -> ApiResult<(StatusCode, Json<Value>)> {
    let req: StartReq = serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(format!("invalid body: {e}")))?;
    let mode = req.mode.unwrap_or_else(|| "stream".into());
    if !["stream", "download"].contains(&mode.as_str()) {
        return Err(ApiError::bad_request("mode must be 'stream' or 'download'"));
    }
    if req.media.as_ref().is_some_and(|m| !["movie", "tv"].contains(&m.kind.as_str()) || m.title.trim().is_empty()) {
        return Err(ApiError::bad_request("media.type must be 'movie' or 'tv' and media.title is required"));
    }
    let from_magnet = req.magnet.as_deref().map(|m| parse_magnet(m).ok_or_else(|| ApiError::bad_request("invalid magnet link"))).transpose()?;
    let (id, magnet_name, mut trackers) = match (from_magnet, req.infohash.as_deref()) {
        (Some(m), _) => m,
        (None, Some(h)) => (clean_hash(h).ok_or_else(|| ApiError::bad_request("invalid infohash"))?, None, Vec::new()),
        (None, None) => return Err(ApiError::bad_request("'magnet' or 'infohash' is required")),
    };
    trackers.extend(req.trackers);
    let wanted = req.files.or(req.file_index.map(|i| vec![i]));
    let tor = &st.torrents;
    if tor.vpn.route() == Route::None {
        return Err(ApiError::needs_vpn());
    }

    let existing = tor.entries.lock().unwrap().get(&id).cloned();
    let entry = match existing {
        Some(e) => {
            {
                let mut m = e.meta.lock().unwrap();
                // Watching a queued download now moves it past the queue.
                m.mode = mode;
                if let Some(p) = req.priority {
                    m.priority = p.clamp(-100, 100);
                }
            }
            if let Some(files) = wanted {
                select_files(&st, &e, files).await?;
            }
            resume_entry(&st, &e).await;
            e
        }
        None => {
            let stream = mode == "stream";
            let e = add(
                &st,
                NewTorrent {
                    id: id.clone(),
                    name: req.name.or(magnet_name).unwrap_or_else(|| id.clone()),
                    mode,
                    media: req.media,
                    trackers,
                    wanted,
                    priority: req.priority.unwrap_or(0),
                    auto: false,
                },
            );
            // A stream starts at once; a download waits for the manager (queue).
            if stream {
                tokio::spawn(bring_up(st.clone(), e.clone()));
            }
            e
        }
    };
    Ok((StatusCode::ACCEPTED, Json(info(tor, &entry))))
}

/// `GET /api/torrents`
pub async fn list(State(st): St) -> Json<Value> {
    let tor = &st.torrents;
    Json(json!({ "vpn": tor.vpn.status(), "torrents": tor.all().iter().map(|e| info(tor, e)).collect::<Vec<_>>() }))
}

/// `GET /api/torrents/:id`
pub async fn get(State(st): St, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    Ok(Json(info(&st.torrents, &*st.torrents.entry(&id)?)))
}

/// `POST /api/torrents/:id/pause`
pub async fn pause(State(st): St, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let e = st.torrents.entry(&id)?;
    e.meta.lock().unwrap().paused = true;
    persist(&st, &e);
    e.stop_announcer();
    if let Some(h) = e.handle() {
        let _ = st.torrents.session.pause(&h).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn resume_entry(st: &Arc<AppState>, e: &Arc<Entry>) {
    let was_paused = std::mem::replace(&mut e.meta.lock().unwrap().paused, false);
    persist(st, e);
    if e.meta.lock().unwrap().completed {
        return;
    }
    match e.handle() {
        Some(h) if was_paused || h.is_paused() => {
            // Tunnel first: peers dialled while it is still down fail at once and sit out a ~10 s backoff.
            let _ = st.torrents.vpn.ensure_up().await;
            st.torrents.start_announcer(e);
            let _ = st.torrents.session.unpause(&h).await;
        }
        Some(_) => {}
        // Not in the engine yet: the manager loop brings it up (and the tunnel with it).
        None => *e.last_try.lock().unwrap() = None,
    }
}

/// `POST /api/torrents/:id/resume`
pub async fn resume(State(st): St, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let e = st.torrents.entry(&id)?;
    resume_entry(&st, &e).await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn select_files(st: &Arc<AppState>, e: &Arc<Entry>, files: Vec<usize>) -> ApiResult<()> {
    {
        let mut m = e.meta.lock().unwrap();
        if m.files.is_empty() {
            // Metadata still resolving: remember the wish.
            m.wanted = Some(files);
            return Ok(());
        }
        if files.is_empty() || files.iter().any(|i| !m.files.iter().any(|f| f.index == *i)) {
            return Err(ApiError::bad_request("'files' must be a non-empty list of file indexes of this torrent"));
        }
        if m.selected.iter().collect::<HashSet<_>>() == files.iter().collect() {
            return Ok(());
        }
        m.selected = files.clone();
        m.completed = false;
    }
    persist(st, e);
    if let Some(h) = e.handle() {
        st.torrents
            .session
            .update_only_files(&h, &files.into_iter().collect())
            .await
            .map_err(|err| ApiError::internal(format!("cannot change files: {err:#}")))?;
        if h.is_paused() && !e.meta.lock().unwrap().paused {
            st.torrents.start_announcer(e);
            let _ = st.torrents.session.unpause(&h).await;
        }
    }
    Ok(())
}

/// `PUT /api/torrents/:id/files` body `{ files: number[] }`
pub async fn put_files(State(st): St, Path(id): Path<String>, body: Bytes) -> ApiResult<StatusCode> {
    #[derive(Deserialize)]
    struct Body {
        files: Vec<usize>,
    }
    let b: Body = serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(format!("invalid body: {e}")))?;
    select_files(&st, &st.torrents.entry(&id)?, b.files).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/torrents/:id?files=1`
pub async fn delete(State(st): St, Path(id): Path<String>, Query(q): Params) -> ApiResult<StatusCode> {
    let tor = &st.torrents;
    let e = tor.entry(&id)?;
    tor.entries.lock().unwrap().remove(&id);
    e.stop_announcer();
    let handle = e.handle.lock().unwrap().take();
    if let Some(h) = handle {
        let _ = tor.session.delete(h.id().into(), false).await;
    }
    st.db.blob_delete(Table::Torrents, &id)?;
    let _ = tokio::fs::remove_file(tor.torrent_file(&id)).await;
    tor.probes.lock().unwrap().retain(|(pid, _), _| *pid != id);
    tor.playing.lock().unwrap().retain(|(pid, _), _| *pid != id);
    let m = e.meta();
    if q.get("files").is_some_and(|v| v == "1") && !m.folder.is_empty() {
        let folder = PathBuf::from(&m.folder);
        for f in &m.files {
            let rel = FsPath::new(&f.path);
            if rel.components().all(|c| matches!(c, Component::Normal(_))) {
                let _ = tokio::fs::remove_file(folder.join(rel)).await;
                // Now-empty sub-folders of the torrent (remove_dir refuses non-empty ones).
                for dir in rel.ancestors().skip(1).filter(|d| !d.as_os_str().is_empty()) {
                    let _ = tokio::fs::remove_dir(folder.join(dir)).await;
                }
            }
        }
        let _ = tokio::fs::remove_dir(&folder).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/torrents/:id/open`: shows the torrent's folder in Explorer on the PC Flimmer runs on,
/// with its first chosen video selected when that file exists.
pub async fn open_folder(State(st): St, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let m = st.torrents.entry(&id)?.meta();
    let folder = PathBuf::from(&m.folder);
    if m.folder.is_empty() || !folder.is_dir() {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "the folder does not exist yet"));
    }
    let video = m
        .files
        .iter()
        .filter(|f| m.selected.contains(&f.index) && is_video(&f.path))
        .filter(|f| FsPath::new(&f.path).components().all(|c| matches!(c, Component::Normal(_))))
        .map(|f| folder.join(&f.path))
        .find(|p| p.is_file());
    open_in_explorer(&folder, video.as_deref()).map_err(|_| ApiError::internal("cannot open the folder"))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(windows)]
fn open_in_explorer(folder: &FsPath, select: Option<&FsPath>) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new("explorer.exe");
    // Explorer wants `/select,"path"` as it is; the default quoting would wrap the whole argument.
    match select {
        Some(file) => cmd.raw_arg(format!("/select,\"{}\"", file.display())),
        None => cmd.arg(folder),
    };
    cmd.spawn().map(drop)
}

#[cfg(not(windows))]
fn open_in_explorer(folder: &FsPath, _select: Option<&FsPath>) -> std::io::Result<()> {
    std::process::Command::new("xdg-open").arg(folder).spawn().map(drop)
}

/// `GET /api/torrents/settings`
pub async fn settings_get(State(st): St) -> ApiResult<Json<TorrentSettings>> {
    load_settings(&st).map(Json)
}

/// `PUT /api/torrents/settings`
pub async fn settings_put(State(st): St, body: Bytes) -> ApiResult<StatusCode> {
    let s: TorrentSettings = serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(format!("invalid body: {e}")))?;
    if !(1..=1000).contains(&s.upload_kbps) {
        return Err(ApiError::bad_request("upload_kbps must be between 1 and 1000"));
    }
    for (name, dir) in [("movies_dir", &s.movies_dir), ("series_dir", &s.series_dir)] {
        if !FsPath::new(dir).is_absolute() {
            return Err(ApiError::bad_request(format!("{name} must be an absolute path")));
        }
    }
    if s.vpn_config.trim().is_empty() {
        return Err(ApiError::bad_request("vpn_config is required"));
    }
    if !(1..=10).contains(&s.max_active) {
        return Err(ApiError::bad_request("max_active must be between 1 and 10"));
    }
    if s.direct && !s.allow_direct {
        return Err(ApiError::bad_request("'direct' needs 'allow_direct'"));
    }
    if let Some(bad) = s.sources_off.iter().find(|id| !crate::tsearch::SOURCES.iter().any(|x| x.id == id.as_str())) {
        return Err(ApiError::bad_request(format!("unknown search source '{bad}'")));
    }
    let data = serde_json::to_string(&s).map_err(|_| ApiError::internal("could not encode settings"))?;
    // Another set of sources: cached searches would show the old one.
    st.torrents.searcher.clear_cache();
    st.db.blob_put(Table::Settings, SETTINGS_KEY, &data)?;
    st.torrents.vpn.set_conf(PathBuf::from(&s.vpn_config));
    st.torrents.vpn.set_direct(s.allow_direct, s.direct);
    st.torrents.session.ratelimits.set_upload_bps(upload_limit(s.upload_kbps));
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /api/torrents/:id/priority` body `{ priority: number }`
pub async fn put_priority(State(st): St, Path(id): Path<String>, body: Bytes) -> ApiResult<StatusCode> {
    #[derive(Deserialize)]
    struct Body {
        priority: i32,
    }
    let b: Body = serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(format!("invalid body: {e}")))?;
    if !(-100..=100).contains(&b.priority) {
        return Err(ApiError::bad_request("priority must be between -100 and 100"));
    }
    let e = st.torrents.entry(&id)?;
    e.meta.lock().unwrap().priority = b.priority;
    persist(&st, &e);
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/torrents/events?since=<id>`
pub async fn events(State(st): St, Query(q): Params) -> ApiResult<Json<Value>> {
    let since = q.get("since").map_or(Ok(0), |v| v.parse::<u64>()).map_err(|_| ApiError::bad_request("'since' must be an event id"))?;
    let events = st.torrents.events_since(since);
    let last = st.torrents.next_event.load(Ordering::SeqCst) - 1;
    Ok(Json(json!({ "events": events, "last": last })))
}

fn file_ref(id: &str, m: &Meta, index: usize) -> Value {
    let f = m.files.iter().find(|f| f.index == index);
    let ep = f.and_then(|f| episode_of(&f.path, m.media.as_ref().and_then(|x| x.season)));
    json!({
        "torrent_id": id, "file": index, "name": f.map(|f| f.path.clone()), "size": f.map(|f| f.size),
        "season": ep.map(|x| x.0), "episode": ep.map(|x| x.1),
        "selected": m.selected.contains(&index), "completed": m.completed,
        "url": format!("/api/torrents/{id}/play?file={index}"),
        "playinfo": format!("/api/torrents/{id}/playinfo?file={index}"),
    })
}

/// Selects a file of a running torrent in addition to the chosen ones and fetches its start first.
async fn also_select(st: &Arc<AppState>, e: &Arc<Entry>, file: usize) -> ApiResult<()> {
    let mut files = e.meta.lock().unwrap().selected.clone();
    if !files.contains(&file) {
        files.push(file);
        select_files(st, e, files).await?;
        resume_entry(st, e).await;
    }
    if !e.meta.lock().unwrap().completed {
        crate::play::prime(st, e, file);
    }
    Ok(())
}

/// `GET /api/torrents/:id/next?file=<i>`: the next episode in a season pack. Selects it for download
/// when it is not chosen yet, so the player can go on with it.
pub async fn next(State(st): St, Path(id): Path<String>, Query(q): Params) -> ApiResult<Json<Value>> {
    let e = st.torrents.entry(&id)?;
    let m = e.meta();
    let current: usize = match q.get("file") {
        Some(v) => v.parse().map_err(|_| ApiError::bad_request("'file' must be a file index"))?,
        None => m.selected.iter().copied().find(|i| m.files.iter().any(|f| f.index == *i && is_video(&f.path))).ok_or_else(|| ApiError::not_found("no video file"))?,
    };
    let (_, _, file) = next_episode(&m.files, m.media.as_ref().and_then(|x| x.season), current)
        .ok_or_else(|| ApiError::not_found("no next episode in this torrent"))?;
    also_select(&st, &e, file).await?;
    Ok(Json(file_ref(&id, &e.meta(), file)))
}

/// The torrent file that holds a movie / an episode, if a known torrent has it.
fn local_file(tor: &Torrents, kind: &str, id: u64, episode: Option<(u32, u32)>) -> Option<(Arc<Entry>, usize)> {
    tor.of_title(kind, id).into_iter().find_map(|e| {
        let m = e.meta();
        let file = match episode {
            Some(ep) => episodes(&m.files, m.media.as_ref().and_then(|x| x.season)).into_iter().find(|(s, n, _)| (*s, *n) == ep).map(|x| x.2)?,
            None => m.selected.iter().copied().find(|i| m.files.iter().any(|f| f.index == *i && is_video(&f.path)))?,
        };
        Some((e, file))
    })
}

/// Whether a known torrent already holds this movie / episode.
pub fn has_local(tor: &Torrents, kind: &str, id: u64, episode: Option<(u32, u32)>) -> bool {
    local_file(tor, kind, id, episode).is_some()
}

/// `GET /api/torrents/local/:type/:id?s=&e=`: where a movie / episode can be played from a torrent
/// that is already there (a download or a season pack).
pub async fn local(State(st): St, Path((ty, id)): Path<(String, String)>, Query(q): Params) -> ApiResult<Json<Value>> {
    let t = crate::api::title_ref(&ty, &id, &q)?;
    let (e, file) = local_file(&st.torrents, t.kind.as_str(), t.id, t.episode).ok_or_else(|| ApiError::not_found("no torrent has this title"))?;
    let mut v = file_ref(&e.id, &e.meta(), file);
    v["state"] = info(&st.torrents, &e)["state"].clone();
    Ok(Json(v))
}

/// Called when watch progress is saved: past the middle of an episode that plays from a season
/// pack, the next episode's file is selected and its start fetched, so "next episode" starts at once.
pub async fn on_progress(st: Arc<AppState>, id: u64, season: u32, episode: u32, position: f64, duration: f64) {
    if duration <= 0.0 || position < duration * 0.5 {
        return;
    }
    let Some((e, file)) = local_file(&st.torrents, "tv", id, Some((season, episode))) else { return };
    let m = e.meta();
    if m.paused {
        return;
    }
    if let Some((_, _, next)) = next_episode(&m.files, m.media.as_ref().and_then(|x| x.season), file)
        && !m.selected.contains(&next)
    {
        let _ = also_select(&st, &e, next).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> TorrentSettings {
        TorrentSettings {
            vpn_config: String::new(),
            movies_dir: r"D:\Flimmer Downloads\Filme".into(),
            series_dir: r"D:\Flimmer Downloads\Serien".into(),
            upload_kbps: 30,
            max_active: 3,
            min_free_gb: 5,
            sources_off: Vec::new(),
            allow_direct: false,
            direct: false,
        }
    }

    #[test]
    fn old_settings_and_torrents_still_load() {
        let s: TorrentSettings = serde_json::from_str(r#"{"vpn_config":"x","movies_dir":"D:\\a","series_dir":"D:\\b","upload_kbps":30}"#).unwrap();
        assert_eq!((s.max_active, s.min_free_gb), (3, 5));
        assert_eq!(s.sources_off, crate::tsearch::default_off());
        assert!(!s.allow_direct && !s.direct);
        let m: Meta = serde_json::from_str(
            r#"{"id":"aa","name":"n","mode":"download","media":null,"trackers":[],"folder":"","files":[],"selected":[],"wanted":null,"paused":false,"completed":false,"added_at":1}"#,
        )
        .unwrap();
        assert_eq!((m.priority, m.auto), (0, false));
    }

    #[test]
    fn maps_pack_files_to_episodes() {
        assert_eq!(episode_of("Show.S01/Show.S01E02.1080p.mkv", None), Some((1, 2)));
        assert_eq!(episode_of("Show/Season 2/Show - 2x07 - Title.mkv", None), Some((2, 7)));
        assert_eq!(episode_of("Dark/Staffel 3/Dark Folge 4.mkv", None), Some((3, 4)));
        assert_eq!(episode_of("[Group] Show - 05 (1080p) [ABCD].mkv", Some(2)), Some((2, 5)));
        assert_eq!(episode_of("Show/Season 1/Episode 12.mp4", None), Some((1, 12)));
        assert_eq!(episode_of("Show.E03.German.720p.mkv", None), Some((1, 3)));
        assert_eq!(episode_of("Movie.2020.1080p.mkv", None), None);

        let pack = files(&[
            ("S01/Show.S01E03.mkv", 9),
            ("S01/Show.S01E01.mkv", 9),
            ("S01/Sample/show.s01e01.sample.mkv", 1),
            ("S01/Show.S01E02.mkv", 9),
            ("S02/Show.S02E01.mkv", 9),
            ("S01/info.nfo", 1),
        ]);
        assert_eq!(episodes(&pack, None), vec![(1, 1, 1), (1, 2, 3), (1, 3, 0), (2, 1, 4)]);
        assert_eq!(next_episode(&pack, None, 1), Some((1, 2, 3)));
        // The last episode of a season leads into the next season of the pack.
        assert_eq!(next_episode(&pack, None, 0), Some((2, 1, 4)));
        assert_eq!(next_episode(&pack, None, 4), None);
        assert_eq!(next_episode(&pack, None, 5), None);
    }

    #[test]
    fn stored_torrents_may_only_name_the_shim_tracker() {
        let shim = shim_url();
        let file = |extra: &str| format!("d8:announce{}:{shim}{extra}4:infod6:lengthi1e4:name1:xee", shim.len()).into_bytes();
        assert!(only_shim_trackers(&file("")));
        assert!(only_shim_trackers(&file(&format!("13:announce-listll{}:{shim}ee", shim.len()))));
        assert!(only_shim_trackers(b"d4:infod6:lengthi1e4:name1:xee"));
        // A real tracker, a web seed, or a broken file: never handed to the engine.
        assert!(!only_shim_trackers(&file("13:announce-listll26:udp://tracker.example:1337ee")));
        assert!(!only_shim_trackers(b"d8:announce26:udp://tracker.example:13374:infod6:lengthi1e4:name1:xee"));
        assert!(!only_shim_trackers(&file("8:url-list18:http://seed.example")));
        assert!(!only_shim_trackers(b"d8:announce3:abc"));
        assert!(!only_shim_trackers(b"not a torrent"));
    }

    #[test]
    fn queue_puts_streams_first_then_priority_then_age() {
        // (stream, priority, added_at, tag)
        let mut list = vec![(false, 0, 10, 0), (false, 5, 30, 1), (true, 0, 40, 2), (false, 0, 5, 3), (false, 5, 20, 4)];
        queue_order(&mut list);
        assert_eq!(list.iter().map(|x| x.3).collect::<Vec<_>>(), vec![2, 4, 1, 3, 0]);
    }

    #[test]
    fn refuses_downloads_that_do_not_fit() {
        let dir = FsPath::new(r"D:\Flimmer Downloads\Filme\X");
        assert!(check_space(Some(100 * GIB), 50 * GIB, 5 * GIB, dir).is_ok());
        assert!(check_space(Some(55 * GIB), 50 * GIB, 5 * GIB, dir).is_ok());
        let err = check_space(Some(54 * GIB), 50 * GIB, 5 * GIB, dir).unwrap_err();
        assert!(err.contains("not enough disk space") && err.contains("50.0 GB needed") && err.contains("54.0 GB free"), "{err}");
        // Unknown free space never blocks.
        assert!(check_space(None, u64::MAX, GIB, dir).is_ok());
        #[cfg(windows)]
        assert!(free_space(&std::env::temp_dir().join("does-not-exist-yet").join("deeper")).is_some_and(|b| b > 0));
    }

    fn media(kind: &str, title: &str, year: Option<u32>, season: Option<u32>, episode: Option<u32>) -> Media {
        Media { kind: kind.into(), id: None, title: title.into(), year, season, episode }
    }

    #[test]
    fn names_download_folders() {
        let s = settings();
        let movie = media("movie", "Mission: Impossible – Fallout", Some(2018), None, None);
        assert_eq!(folder_for(&s, Some(&movie), "x"), PathBuf::from(r"D:\Flimmer Downloads\Filme\Mission Impossible – Fallout (2018)"));
        let ep = media("tv", "Was wäre wenn...?", None, Some(2), Some(5));
        assert_eq!(folder_for(&s, Some(&ep), "x"), PathBuf::from(r"D:\Flimmer Downloads\Serien\Was wäre wenn\Staffel 02"));
        assert_eq!(folder_for(&s, None, "Sintel.2010.1080p"), PathBuf::from(r"D:\Flimmer Downloads\Filme\Sintel.2010.1080p"));
    }

    #[test]
    fn sanitizes_path_components() {
        assert_eq!(sanitize(r#"a<b>c:d"e/f\g|h?i*j"#), "a b c d e f g h i j");
        assert_eq!(sanitize("  ..Title..  "), "Title");
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("com1.txt"), "_com1.txt");
        assert_eq!(sanitize("???"), "Unbenannt");
    }

    fn files(list: &[(&str, u64)]) -> Vec<FileMeta> {
        list.iter().enumerate().map(|(index, (p, size))| FileMeta { index, path: p.to_string(), size: *size }).collect()
    }

    #[test]
    fn chooses_files() {
        let pack = files(&[
            ("Show.S01/Show.S01E01.1080p.mkv", 900),
            ("Show.S01/Show.S01E02.1080p.mkv", 950),
            ("Show.S01/Sample/show.s01e02.sample.mkv", 20),
            ("Show.S01/info.nfo", 1),
        ]);
        let ep2 = media("tv", "Show", None, Some(1), Some(2));
        assert_eq!(choose_files(&pack, None, Some(&ep2)), vec![1]);
        let season = media("tv", "Show", None, Some(1), None);
        assert_eq!(choose_files(&pack, None, Some(&season)), vec![0, 1]);
        assert_eq!(choose_files(&pack, Some(&[0, 99]), Some(&ep2)), vec![0]);

        let movie = files(&[("Sintel/poster.jpg", 5), ("Sintel/sintel.mp4", 129_000_000), ("Sintel/sintel.en.srt", 2)]);
        assert_eq!(choose_files(&movie, None, None), vec![1]);
        assert_eq!(choose_files(&files(&[("a.iso", 7), ("b.txt", 1)]), None, None), vec![0]);
    }
}
