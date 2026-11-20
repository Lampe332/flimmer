//! Flimmer server as a library: the standalone binary and the desktop app both start it,
//! the desktop app additionally injects a [`sniff::Sniffer`].

mod api;
mod db;
mod error;
mod hls;
mod model;
mod scan;
pub mod sniff;
mod stream;
mod tmdb;

use std::{
    future::Future,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    Router,
    http::{HeaderValue, Method, header},
};
use serde::Deserialize;
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
};

pub const DEFAULT_PORT: u16 = 43300;
const DEV_ORIGIN: &str = "http://localhost:5173";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Deserialize)]
struct Config {
    tmdb_read_token: String,
    #[serde(default = "default_language")]
    language: String,
    #[serde(default = "default_region")]
    region: String,
}

fn default_language() -> String {
    "de-DE".into()
}

fn default_region() -> String {
    "DE".into()
}

pub struct Options {
    /// Folder with `config.json`, `providers.json`, `data/` and `web/dist`.
    pub root: PathBuf,
    pub port: u16,
    pub sniffer: Option<Arc<dyn sniff::Sniffer>>,
}

/// A bound, not yet serving server.
pub struct Server {
    listener: tokio::net::TcpListener,
    app: Router,
    pub url: String,
}

fn load_config(root: &Path) -> Result<Config, String> {
    let path = root.join("config.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    // Deliberately do not include serde's message: it could quote parts of the token.
    let cfg: Config = serde_json::from_str(&text).map_err(|_| format!("{} is not valid config JSON", path.display()))?;
    if cfg.tmdb_read_token.trim().is_empty() {
        return Err(format!("tmdb_read_token is empty in {}", path.display()));
    }
    Ok(cfg)
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
    let cfg = load_config(&opts.root)?;
    let providers = load_providers(&opts.root)?;

    let data_dir = opts.root.join("data");
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

    let state = Arc::new(api::AppState {
        tmdb: tmdb::Tmdb::new(cfg.tmdb_read_token, cfg.language, cfg.region),
        db,
        providers,
        sniffer: opts.sniffer,
        scans: scan::Scans::default(),
        streams,
    });

    let cors = CorsLayer::new()
        .allow_origin(HeaderValue::from_static(DEV_ORIGIN))
        .allow_methods([Method::GET, Method::PUT, Method::POST, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE]);

    let app = api::router(state).merge(web_service(&opts.root)).layer(cors);

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, opts.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("cannot bind {addr}: {e}"))?;
    Ok(Server { listener, app, url: format!("http://{addr}") })
}

impl Server {
    pub async fn serve(self, shutdown: impl Future<Output = ()> + Send + 'static) -> Result<(), String> {
        axum::serve(self.listener, self.app)
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|e| format!("server error: {e}"))
    }
}
