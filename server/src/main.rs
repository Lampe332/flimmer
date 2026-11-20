mod api;
mod db;
mod error;
mod model;
mod tmdb;

use std::{
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::ExitCode,
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

const DEFAULT_PORT: u16 = 43300;
const DEV_ORIGIN: &str = "http://localhost:5173";

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

struct Args {
    root: PathBuf,
    port: u16,
    open: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut root = None;
    let mut port = DEFAULT_PORT;
    let mut open = false;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--root" => root = Some(PathBuf::from(it.next().ok_or("--root needs a path")?)),
            "--port" => {
                port = it
                    .next()
                    .ok_or("--port needs a number")?
                    .parse()
                    .map_err(|_| "--port must be a number between 1 and 65535")?;
            }
            "--open" => open = true,
            "-h" | "--help" => {
                println!("Usage: flimmer [--root <path>] [--port <port>] [--open]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let root = match root {
        Some(r) => r,
        None => std::env::current_dir().map_err(|e| format!("cannot read current directory: {e}"))?,
    };
    Ok(Args { root, port, open })
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

fn open_browser(url: &str) {
    #[cfg(windows)]
    let res = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(target_os = "macos")]
    let res = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let res = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = res {
        eprintln!("could not open browser: {e}");
    }
}

async fn run() -> Result<(), String> {
    let args = parse_args()?;
    let cfg = load_config(&args.root)?;
    let providers = load_providers(&args.root)?;

    let data_dir = args.root.join("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("cannot create {}: {e}", data_dir.display()))?;
    let db_path = data_dir.join("flimmer.db");
    let db = db::Db::open(&db_path).map_err(|e| format!("cannot open {}: {e}", db_path.display()))?;

    let state = Arc::new(api::AppState {
        tmdb: tmdb::Tmdb::new(cfg.tmdb_read_token, cfg.language, cfg.region),
        db,
        providers,
    });

    let cors = CorsLayer::new()
        .allow_origin(HeaderValue::from_static(DEV_ORIGIN))
        .allow_methods([Method::GET, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE]);

    let app = api::router(state).merge(web_service(&args.root)).layer(cors);

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, args.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("cannot bind {addr}: {e}"))?;
    let url = format!("http://{addr}");
    println!("Flimmer running at {url}");
    if args.open {
        open_browser(&url);
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| format!("server error: {e}"))
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("flimmer: {e}");
            ExitCode::FAILURE
        }
    }
}
