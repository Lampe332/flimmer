//! Flimmer server as a library: the standalone binary and the desktop app both start it,
//! the desktop app additionally injects a [`sniff::Sniffer`].

mod api;
mod autodl;
mod benc;
mod db;
mod dht;
mod error;
mod guard;
mod hls;
mod model;
pub mod paths;
mod play;
mod progress;
mod release;
mod scan;
pub mod setup;
pub mod sniff;
mod stream;
mod tmdb;
mod torrent;
mod tracker;
mod tsearch;
mod vpn;
mod wg;

use std::{
    future::Future,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{Extension, Router, extract::DefaultBodyLimit};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use tower_http::services::{ServeDir, ServeFile};

pub const DEFAULT_PORT: u16 = 43300;
/// No request body is larger than an export file with a long watchlist.
const BODY_MAX: usize = 1024 * 1024;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Locks `m`, also after a panic in another holder (the data is still usable for everything here).
fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Legacy `config.json` (installs before 1.0). Optional now: the token lives in the database, set by the
/// first-run setup; an old file's token is copied over once.
#[derive(Deserialize)]
struct Config {
    #[serde(default)]
    tmdb_read_token: String,
    #[serde(default = "default_language")]
    language: String,
    #[serde(default = "default_region")]
    region: String,
}

impl Default for Config {
    fn default() -> Self {
        Self { tmdb_read_token: String::new(), language: default_language(), region: default_region() }
    }
}

fn default_language() -> String {
    "de-DE".into()
}

fn default_region() -> String {
    "DE".into()
}

pub struct Options {
    /// Read-only resources: `providers.json`, `web/dist`, `tools/` (see [`paths`]).
    pub root: PathBuf,
    /// Per-user data: `data/` below it, plus an optional legacy `config.json`. Development: same as `root`.
    pub home: PathBuf,
    pub port: u16,
    pub sniffer: Option<Arc<dyn sniff::Sniffer>>,
    /// Development only: also accept the Vite dev server (`localhost:5173`, which proxies `/api`) as host and origin.
    pub dev: bool,
}

/// A bound, not yet serving server.
pub struct Server {
    listener: tokio::net::TcpListener,
    app: Router,
    state: Arc<api::AppState>,
    pub url: String,
}

/// The legacy `config.json` in `home` (else `root`), or defaults when there is none.
/// A file that is not valid JSON still stops the start, so a typo does not silently drop the token.
fn load_config(root: &Path, home: &Path) -> Result<Config, String> {
    let Some(path) = [home, root].iter().map(|d| d.join("config.json")).find(|p| p.is_file()) else {
        return Ok(Config::default());
    };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    // Deliberately do not include serde's message: it could quote parts of the token.
    serde_json::from_str(&text).map_err(|_| format!("{} is not valid config JSON", path.display()))
}

/// The token to use: the stored one, else a real one from a legacy `config.json` (copied into the database once).
fn initial_token(db: &db::Db, cfg: &Config) -> String {
    if let Some(t) = setup::stored_token(db) {
        return t;
    }
    let legacy = cfg.tmdb_read_token.trim();
    if legacy.is_empty() || legacy.starts_with("PUT_YOUR") {
        return String::new();
    }
    let _ = setup::store_token(db, legacy);
    // An install from before the first-run flow: it is set up already, so no onboarding and no tour.
    setup::mark_set_up(db);
    legacy.to_owned()
}

fn load_providers(root: &Path) -> Result<Vec<api::Provider>, String> {
    let path = root.join("providers.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("invalid {}: {e}", path.display()))
}

fn web_service(root: &Path) -> Router {
    let dist = root.join("web").join("dist");
    let index = dist.join("index.html");
    if index.is_file() {
        Router::new().fallback_service(ServeDir::new(&dist).fallback(ServeFile::new(index)))
    } else {
        Router::new().fallback(|| async { "Flimmer UI not built yet (run npm run build in web/)" })
    }
}

/// Loads config and data, cleans old cache and binds `127.0.0.1:port`.
pub async fn bind(opts: Options) -> Result<Server, String> {
    paths::init(&opts.root, &opts.home);
    let cfg = load_config(&opts.root, &opts.home)?;
    let providers = load_providers(&opts.root)?;

    let data_dir = paths::data();
    let cache_dir = data_dir.join("cache");
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("cannot create {}: {e}", cache_dir.display()))?;
    let db_path = data_dir.join("flimmer.db");
    let db = db::Db::open(&db_path).map_err(|e| format!("cannot open {}: {e}", db_path.display()))?;

    let streams = stream::Streams::new(cache_dir)?;
    streams.housekeeping();
    // Captured stream URLs carry short-lived tokens; day-old rows are dead weight.
    for table in [db::Table::Scans, db::Table::Streams] {
        let _ = db.blob_prune(table, db::now_ms() - DAY_MS);
    }

    // The engine keeps its state below `<home>/data` (torrents, default VPN file).
    let torrents = torrent::Torrents::new(opts.home.clone(), opts.port, &torrent::initial_settings(&db, &opts.home)).await?;

    let token = initial_token(&db, &cfg);
    let guard = guard::Guard::new(&db, opts.port, opts.dev);
    let state = Arc::new(api::AppState {
        guard,
        tmdb: tmdb::Tmdb::new(token, cfg.language, cfg.region),
        db,
        providers,
        sniffer: opts.sniffer,
        scans: scan::Scans::default(),
        streams,
        torrents,
        autodl: autodl::JobState::default(),
    });
    if let Ok(s) = scan::load_settings(&state) {
        state.scans.set_parallel(s.parallel as usize);
    }
    torrent::spawn_manager(state.clone());
    autodl::spawn(state.clone());

    // No CORS headers at all: the UI is served from the same origin, so no other origin gets to read answers.
    let app = guard::router(state.clone())
        .merge(setup::router(state.clone()))
        .merge(api::router(state.clone()))
        .merge(web_service(&opts.root))
        .layer(DefaultBodyLimit::max(BODY_MAX))
        .layer(axum::middleware::from_fn_with_state(state.clone(), guard::middleware));

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, opts.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("cannot bind {addr}: {e}"))?;
    Ok(Server { listener, app, state, url: format!("http://{addr}") })
}

type Service = axum::extract::connect_info::IntoMakeServiceWithConnectInfo<Router, SocketAddr>;

fn service(app: &Router, via: guard::Via) -> Service {
    app.clone().layer(Extension(via)).into_make_service_with_connect_info::<SocketAddr>()
}

/// "Handy & Tablet im WLAN": listens on the PC's private LAN address while the setting is on, and follows
/// the setting and the address (DHCP) while the server runs. Never binds `0.0.0.0`.
async fn lan_listener(state: Arc<api::AppState>, app: Router, port: u16) {
    let guard = &state.guard;
    let mut current: Option<(Ipv4Addr, CancellationToken)> = None;
    loop {
        let want = guard.lan_target();
        if want != current.as_ref().map(|c| c.0) {
            if let Some((_, stop)) = current.take() {
                stop.cancel();
                // The old listener closes its socket a moment after the signal.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            guard.set_bound(None, None);
            if let Some(ip) = want {
                match tokio::net::TcpListener::bind(SocketAddr::from((ip, port))).await {
                    Ok(listener) => {
                        let stop = CancellationToken::new();
                        let signal = stop.clone();
                        let svc = service(&app, guard::Via::Lan);
                        tokio::spawn(async move {
                            let _ = axum::serve(listener, svc).with_graceful_shutdown(async move { signal.cancelled().await }).await;
                        });
                        guard.set_bound(Some(ip), None);
                        current = Some((ip, stop));
                    }
                    Err(e) => guard.set_bound(None, Some(format!("cannot listen on the network address: {}", e.kind()))),
                }
            }
        }
        tokio::select! {
            _ = guard.changed.notified() => {}
            _ = tokio::time::sleep(std::time::Duration::from_secs(15)) => {}
        }
    }
}

impl Server {
    /// The address to open the UI with: a one-time ticket that gives the page its session cookie.
    /// Without it (or a paired device token) the API answers `401`.
    pub fn entry_url(&self, valid: std::time::Duration) -> String {
        format!("{}/api/auth/enter?t={}", self.url, self.state.guard.ticket(valid))
    }

    pub async fn serve(self, shutdown: impl Future<Output = ()> + Send + 'static) -> Result<(), String> {
        let port = self.listener.local_addr().map_err(|e| format!("server error: {e}"))?.port();
        let lan = tokio::spawn(lan_listener(self.state.clone(), self.app.clone(), port));
        let res = axum::serve(self.listener, service(&self.app, guard::Via::Loopback))
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|e| format!("server error: {e}"));
        lan.abort();
        res
    }
}
