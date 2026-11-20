//! Torrent engine (embedded librqbit) behind the VPN. The engine's only network paths are the SOCKS
//! server of [`crate::vpn`] (peers) and the tracker shim of [`crate::tracker`]; DHT, local discovery,
//! incoming connections and its own tracker traffic are off. Seeding stops when a download is complete.

use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU32,
    path::{Component, Path as FsPath, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
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
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ConnectionOptions, ManagedTorrent, PeerConnectionOptions,
    Session, SessionOptions, TorrentStatsState, limits::LimitsConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::{
    api::{AppState, Params, St},
    db::{Table, now_ms},
    error::{ApiError, ApiResult},
    release::{self, clean_hash, parse_magnet},
    scan::iso,
    tracker::{self, SHIM_INTERVAL, Swarm},
    tsearch::Searcher,
    vpn::{SHIM_HOST, Vpn},
};

const SETTINGS_KEY: &str = "torrents";
const METADATA_TIMEOUT: Duration = Duration::from_secs(120);
const RETRY_EVERY: Duration = Duration::from_secs(15);
/// The tunnel closes this long after the last torrent stopped needing it.
const TUNNEL_LINGER: Duration = Duration::from_secs(5);
const VIDEO_EXT: &[&str] = &["mkv", "mp4", "m4v", "avi", "mov", "webm", "ts", "m2ts", "wmv", "mpg", "mpeg"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentSettings {
    /// WireGuard config of the VPN (read on every tunnel start, never copied).
    pub vpn_config: String,
    pub movies_dir: String,
    pub series_dir: String,
    /// Upload limit while downloading, KiB/s (1..=1000).
    pub upload_kbps: u32,
}

impl TorrentSettings {
    fn defaults(root: &FsPath) -> Self {
        Self {
            vpn_config: root.join("data").join("vpn").join("proton.conf").to_string_lossy().into_owned(),
            movies_dir: r"D:\Flimmer Downloads\Filme".into(),
            series_dir: r"D:\Flimmer Downloads\Serien".into(),
            upload_kbps: 30,
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
    entries: Mutex<HashMap<String, Arc<Entry>>>,
    peer_id: [u8; 20],
    pub(crate) probes: Mutex<HashMap<(String, usize), Arc<crate::play::Probe>>>,
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
        let vpn = Vpn::new(PathBuf::from(&settings.vpn_config), port).await?;
        let data = root.join("data").join("torrents");
        std::fs::create_dir_all(&data).map_err(|e| format!("cannot create {}: {e}", data.display()))?;
        let opts = SessionOptions {
            dht: None,
            listen: None,
            persistence: None,
            fastresume: false,
            disable_local_service_discovery: true,
            ipv4_only: true,
            peer_limit: Some(50),
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
        let mut peer_id = *b"-FL0100-000000000000";
        peer_id[8..].copy_from_slice(&rand::random::<[u8; 12]>().map(|b| b'0' + b % 10));
        Ok(Arc::new(Self {
            vpn,
            searcher: Searcher::new()?,
            root,
            port,
            session,
            entries: Mutex::default(),
            peer_id,
            probes: Mutex::default(),
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
        let trackers = tracker::tracker_list(&e.meta.lock().unwrap().trackers);
        tokio::spawn(tracker::run(self.vpn.clone(), hash_bytes(&e.id), self.peer_id, trackers, e.swarm.clone(), token));
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
    *e.error.lock().unwrap() = result.err();
    e.busy.store(false, Ordering::SeqCst);
}

async fn bring_up_inner(st: &Arc<AppState>, e: &Arc<Entry>) -> Result<(), String> {
    let tor = &st.torrents;
    tor.vpn.ensure_up().await?;
    tor.start_announcer(e);
    let trackers = Some(vec![shim_url()]);

    let path = tor.torrent_file(&e.id);
    let known = tokio::fs::read(&path).await.ok().filter(|_| !e.meta.lock().unwrap().files.is_empty());
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
    Ok(())
}

/// Background loop: retries, completion (stops seeding) and the tunnel's lifetime.
pub fn spawn_manager(st: Arc<AppState>) {
    for data in st.db.blob_all(Table::Torrents).unwrap_or_default() {
        if let Ok(meta) = serde_json::from_str::<Meta>(&data) {
            st.torrents.entries.lock().unwrap().insert(meta.id.clone(), Entry::new(meta));
        }
    }
    tokio::spawn(async move {
        let tor = st.torrents.clone();
        let mut idle_since: Option<Instant> = None;
        loop {
            let mut active = 0;
            for e in tor.all() {
                let m = e.meta();
                if m.completed || m.paused {
                    continue;
                }
                match e.handle() {
                    Some(h) => {
                        let stats = h.stats();
                        if stats.finished {
                            // Done: stop all peer traffic for it (no seeding).
                            let _ = tor.session.pause(&h).await;
                            e.stop_announcer();
                            e.meta.lock().unwrap().completed = true;
                            persist(&st, &e);
                            continue;
                        }
                        if matches!(stats.state, TorrentStatsState::Error) {
                            *e.error.lock().unwrap() = stats.error;
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
    let state = if m.completed {
        "completed"
    } else if m.paused {
        "paused"
    } else if tor.vpn.is_blocked() {
        "vpn_blocked"
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
    json!({
        "id": m.id, "name": m.name, "state": state, "mode": m.mode, "media": m.media,
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

    let existing = tor.entries.lock().unwrap().get(&id).cloned();
    let entry = match existing {
        Some(e) => {
            if mode == "download" {
                e.meta.lock().unwrap().mode = mode;
            }
            if let Some(files) = wanted {
                select_files(&st, &e, files).await?;
            }
            resume_entry(&st, &e).await;
            e
        }
        None => {
            let e = Entry::new(Meta {
                id: id.clone(),
                name: req.name.or(magnet_name).unwrap_or_else(|| id.clone()),
                mode,
                media: req.media,
                trackers,
                folder: String::new(),
                files: Vec::new(),
                selected: Vec::new(),
                wanted,
                paused: false,
                completed: false,
                added_at: now_ms(),
            });
            tor.entries.lock().unwrap().insert(id, e.clone());
            persist(&st, &e);
            tokio::spawn(bring_up(st.clone(), e.clone()));
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

async fn select_files(st: &Arc<AppState>, e: &Arc<Entry>, files: Vec<usize>) -> ApiResult<()> {
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
    let data = serde_json::to_string(&s).map_err(|_| ApiError::internal("could not encode settings"))?;
    st.db.blob_put(Table::Settings, SETTINGS_KEY, &data)?;
    st.torrents.vpn.set_conf(PathBuf::from(&s.vpn_config));
    st.torrents.session.ratelimits.set_upload_bps(upload_limit(s.upload_kbps));
    Ok(StatusCode::NO_CONTENT)
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
        }
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
