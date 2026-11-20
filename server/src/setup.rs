//! First-run setup and app-level settings that live next to the data, not in the install folder:
//! the TMDB token (stored in SQLite, never sent back to the UI), the "onboarding done" flag, the UI preferences,
//! the imported WireGuard file, cache sizes, opening fixed help pages in the system browser and a full reset.

use std::{path::Path as FsPath, sync::Arc};

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::StatusCode,
    routing::{delete, get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    api::{AppState, St},
    db::{Db, Table},
    error::{ApiError, ApiResult},
    paths,
    tmdb::TokenProblem,
    torrent, wg,
};

const TOKEN_KEY: &str = "tmdb_token";
const SETUP_KEY: &str = "setup";
const PREFS_KEY: &str = "prefs";
/// UI preferences are a small object; anything bigger is not from the UI.
const PREFS_MAX: usize = 16 * 1024;
/// A WireGuard file is a few hundred bytes.
const VPN_MAX: usize = 8 * 1024;
/// Marker the desktop app looks for on start: delete the source-search browser profile, then this file.
pub const CLEAR_SCAN_PROFILE: &str = "clear-scan-profile";

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/setup", get(status))
        .route("/api/setup/tmdb", put(token_put))
        .route("/api/setup/done", post(done))
        .route("/api/setup/vpn", put(vpn_put).delete(vpn_delete))
        .route("/api/setup/open", post(open_page))
        .route("/api/setup/reset", post(reset))
        .route("/api/prefs", get(prefs_get).put(prefs_put))
        .route("/api/export", post(export))
        .route("/api/import", post(import))
        .route("/api/maintenance", get(maintenance))
        .route("/api/maintenance/cache", delete(cache_clear))
        .with_state(state)
}

/// The stored token, if any.
pub fn stored_token(db: &Db) -> Option<String> {
    db.blob_get(Table::Settings, TOKEN_KEY).ok().flatten().filter(|t| !t.trim().is_empty())
}

pub fn store_token(db: &Db, token: &str) -> ApiResult<()> {
    db.blob_put(Table::Settings, TOKEN_KEY, token)
}

/// For installs that were working before the first-run flow existed: onboarding done, tour seen.
pub fn mark_set_up(db: &Db) {
    let _ = db.blob_put(Table::Settings, SETUP_KEY, &json!({ "onboarded": true }).to_string());
    if db.blob_get(Table::Settings, PREFS_KEY).ok().flatten().is_none() {
        let _ = db.blob_put(Table::Settings, PREFS_KEY, &json!({ "tourDone": true }).to_string());
    }
}

fn onboarded(db: &Db) -> bool {
    db.blob_get(Table::Settings, SETUP_KEY)
        .ok()
        .flatten()
        .and_then(|d| serde_json::from_str::<Value>(&d).ok())
        .and_then(|v| v.get("onboarded").and_then(Value::as_bool))
        .unwrap_or(false)
}

/// Default download folders for a fresh install: `Videos\Flimmer\Filme|Serien` of the current user.
fn default_download_dirs() -> (String, String) {
    let base = std::env::var_os("USERPROFILE")
        .map(|p| FsPath::new(&p).join("Videos").join("Flimmer"))
        .unwrap_or_else(|| paths::data().join("Downloads"));
    let s = |name: &str| base.join(name).to_string_lossy().into_owned();
    (s("Filme"), s("Serien"))
}

/// `GET /api/setup`: what the first-run flow needs. The token itself never leaves the server.
async fn status(State(st): St) -> ApiResult<Json<Value>> {
    let (movies, series) = default_download_dirs();
    Ok(Json(json!({
        "tmdb": if st.tmdb.has_token() { "set" } else { "missing" },
        "onboarded": onboarded(&st.db),
        "vpn_file": paths::vpn_file().is_file(),
        "version": env!("CARGO_PKG_VERSION"),
        "defaults": { "movies_dir": movies, "series_dir": series },
    })))
}

#[derive(Deserialize)]
struct TokenBody {
    token: String,
}

/// Rough shape check before asking TMDB, so the most common mix-up gets its own message.
fn token_shape(token: &str) -> Result<(), (&'static str, &'static str)> {
    if token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(("api_key", "this is the short API key; Flimmer needs the long API Read Access Token"));
    }
    let jwt = token.split('.').count() == 3
        && token.len() >= 100
        && token.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    if !jwt {
        return Err(("format", "this does not look like a TMDB API Read Access Token"));
    }
    Ok(())
}

fn coded(status: StatusCode, code: &'static str, message: &str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message, "code": code })))
}

/// `PUT /api/setup/tmdb {token}`: checks the token with TMDB and only stores it when TMDB accepts it.
async fn token_put(State(st): St, Json(body): Json<TokenBody>) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    // Pasted tokens often come with spaces or line breaks around (or inside) them.
    let token: String = body.token.chars().filter(|c| !c.is_whitespace()).collect();
    if let Err((code, msg)) = token_shape(&token) {
        return Err(coded(StatusCode::BAD_REQUEST, code, msg));
    }
    match st.tmdb.try_token(&token).await {
        Ok(()) => {}
        Err(TokenProblem::Rejected) => return Err(coded(StatusCode::UNPROCESSABLE_ENTITY, "rejected", "TMDB does not accept this token")),
        Err(TokenProblem::Unreachable) => return Err(coded(StatusCode::BAD_GATEWAY, "unreachable", "TMDB is not reachable")),
    }
    store_token(&st.db, &token).map_err(|_| coded(StatusCode::INTERNAL_SERVER_ERROR, "store", "could not save the token"))?;
    st.tmdb.set_token(token);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/setup/done`: the first-run flow is finished (it does not show again on its own).
async fn done(State(st): St) -> ApiResult<StatusCode> {
    st.db.blob_put(Table::Settings, SETUP_KEY, &json!({ "onboarded": true }).to_string())?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct VpnBody {
    text: String,
}

/// Points the torrent settings at `path` (keeps folders and upload limit).
async fn use_vpn_file(st: &Arc<AppState>, path: &FsPath) -> ApiResult<()> {
    let mut s = torrent::load_settings(st)?;
    s.vpn_config = path.to_string_lossy().into_owned();
    let body = serde_json::to_vec(&s).map_err(|_| ApiError::internal("could not encode settings"))?;
    torrent::settings_put(State(st.clone()), Bytes::from(body)).await.map(|_| ())
}

/// `PUT /api/setup/vpn {text}`: imports a WireGuard file into Flimmer's data folder.
/// The content is checked, written and never echoed; errors never quote it.
async fn vpn_put(State(st): St, Json(body): Json<VpnBody>) -> ApiResult<StatusCode> {
    if body.text.len() > VPN_MAX {
        return Err(ApiError::bad_request("this is not a WireGuard file (too large)"));
    }
    wg::WgConfig::parse(&body.text).map_err(ApiError::bad_request)?;
    let file = paths::vpn_file();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|_| ApiError::internal("cannot create the VPN folder"))?;
    }
    std::fs::write(&file, body.text.as_bytes()).map_err(|_| ApiError::internal("cannot save the VPN file"))?;
    // A running tunnel still uses the old keys: close it, the next torrent opens a new one.
    st.torrents.vpn.down();
    use_vpn_file(&st, &file).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/setup/vpn`: removes the imported file; torrents stay off until a new one is imported.
async fn vpn_delete(State(st): St) -> ApiResult<StatusCode> {
    let file = paths::vpn_file();
    if file.is_file() {
        std::fs::remove_file(&file).map_err(|_| ApiError::internal("cannot remove the VPN file"))?;
    }
    st.torrents.vpn.down();
    use_vpn_file(&st, &file).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct OpenBody {
    page: String,
}

/// Fixed pages only: the UI can not make Flimmer open arbitrary addresses.
fn page_url(page: &str) -> Option<&'static str> {
    Some(match page {
        "tmdb_signup" => "https://www.themoviedb.org/signup",
        "tmdb_api" => "https://www.themoviedb.org/settings/api",
        "proton_wireguard" => "https://account.protonvpn.com/downloads",
        "proton_help" => "https://protonvpn.com/support/wireguard-configurations",
        // Licenses and sources of what Flimmer ships (settings, "Über Flimmer").
        "tmdb" => "https://www.themoviedb.org/",
        "ffmpeg" => "https://www.gyan.dev/ffmpeg/builds/",
        "ffmpeg_source" => "https://github.com/FFmpeg/FFmpeg",
        "ubol" => "https://github.com/uBlockOrigin/uBOL-home",
        "librqbit" => "https://github.com/ikatson/rqbit",
        "boringtun" => "https://github.com/cloudflare/boringtun",
        "hlsjs" => "https://github.com/video-dev/hls.js",
        "tork" => "https://github.com/melqtx/tork",
        _ => return None,
    })
}

/// `POST /api/setup/open {page}`: opens a help page in the system browser (the app window has no tabs).
async fn open_page(Json(body): Json<OpenBody>) -> ApiResult<StatusCode> {
    let url = page_url(&body.page).ok_or_else(|| ApiError::bad_request("unknown page"))?;
    open_url(url).map_err(|_| ApiError::internal("cannot open the browser"))?;
    Ok(StatusCode::NO_CONTENT)
}

fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No shell involved: the URL goes straight to the default handler.
        std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url])
            .creation_flags(0x0800_0000)
            .spawn()
            .map(|_| ())
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open").arg(url).spawn().map(|_| ())
    }
}

/// `GET /api/prefs`: the UI preferences object (`{}` before the first save).
async fn prefs_get(State(st): St) -> ApiResult<Json<Value>> {
    let v = st
        .db
        .blob_get(Table::Settings, PREFS_KEY)?
        .and_then(|d| serde_json::from_str::<Value>(&d).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    Ok(Json(v))
}

/// `PUT /api/prefs`: stores the UI preferences. They live here (not only in the browser storage) so they survive
/// a port change and are part of export/import.
async fn prefs_put(State(st): St, body: Bytes) -> ApiResult<StatusCode> {
    if body.len() > PREFS_MAX {
        return Err(ApiError::bad_request("prefs are too large"));
    }
    let v: Value = serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("prefs must be JSON"))?;
    if !v.is_object() {
        return Err(ApiError::bad_request("prefs must be a JSON object"));
    }
    st.db.blob_put(Table::Settings, PREFS_KEY, &v.to_string())?;
    Ok(StatusCode::NO_CONTENT)
}

const EXPORT_KIND: &str = "flimmer-settings";

/// Everything "Exportieren" writes: preferences, source and torrent settings (folders, upload; not the VPN
/// file) and the watchlist. Never the TMDB token or VPN keys.
fn export_value(st: &Arc<AppState>) -> ApiResult<Value> {
    let prefs = st.db.blob_get(Table::Settings, PREFS_KEY)?.and_then(|d| serde_json::from_str::<Value>(&d).ok());
    let scan = crate::scan::load_settings(st)?;
    let tor = torrent::load_settings(st)?;
    Ok(json!({
        "kind": EXPORT_KIND,
        "version": 1,
        "app_version": env!("CARGO_PKG_VERSION"),
        "prefs": prefs.unwrap_or_else(|| json!({})),
        "sources": scan,
        "torrents": { "movies_dir": tor.movies_dir, "series_dir": tor.series_dir, "upload_kbps": tor.upload_kbps },
        "watchlist": st.db.list()?,
    }))
}

/// A free file name like `Flimmer-Einstellungen-2026-11-20.json` (then `-2`, `-3` …) in `dir`.
fn export_path(dir: &FsPath) -> std::path::PathBuf {
    let day = crate::scan::iso(crate::db::now_ms());
    let day = day.get(..10).unwrap_or("export");
    (1..1000)
        .map(|n| if n == 1 { format!("Flimmer-Einstellungen-{day}.json") } else { format!("Flimmer-Einstellungen-{day}-{n}.json") })
        .map(|name| dir.join(name))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(format!("Flimmer-Einstellungen-{day}.json")))
}

/// `POST /api/export`: writes the export into the user's Downloads folder and answers with its path.
/// (A browser download inside the app window would land somewhere the user does not see.)
async fn export(State(st): St) -> ApiResult<Json<Value>> {
    let value = export_value(&st)?;
    let dir = std::env::var_os("USERPROFILE")
        .map(|p| FsPath::new(&p).join("Downloads"))
        .filter(|d| d.is_dir())
        .unwrap_or_else(paths::data);
    let path = export_path(&dir);
    let text = serde_json::to_string_pretty(&value).map_err(|_| ApiError::internal("could not encode the export"))?;
    std::fs::write(&path, text).map_err(|_| ApiError::internal("cannot write the export file"))?;
    Ok(Json(json!({ "path": path.to_string_lossy() })))
}

/// `POST /api/import`: applies an export. Each part is checked like its own settings call; the watchlist is merged.
async fn import(State(st): St, body: Bytes) -> ApiResult<Json<Value>> {
    let v: Value = serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("this is not a Flimmer export (no JSON)"))?;
    if v.get("kind").and_then(Value::as_str) != Some(EXPORT_KIND) {
        return Err(ApiError::bad_request("this is not a Flimmer export"));
    }
    if let Some(prefs) = v.get("prefs").filter(|p| p.is_object()) {
        prefs_put(State(st.clone()), Bytes::from(prefs.to_string())).await?;
    }
    if let Some(sources) = v.get("sources") {
        crate::scan::settings_put(State(st.clone()), Bytes::from(sources.to_string())).await?;
    }
    if let Some(t) = v.get("torrents") {
        let mut s = torrent::load_settings(&st)?;
        let text = |k: &str| t.get(k).and_then(Value::as_str).map(str::to_owned);
        s.movies_dir = text("movies_dir").unwrap_or(s.movies_dir);
        s.series_dir = text("series_dir").unwrap_or(s.series_dir);
        s.upload_kbps = t.get("upload_kbps").and_then(Value::as_u64).map_or(s.upload_kbps, |n| n.min(u32::MAX as u64) as u32);
        let body = serde_json::to_vec(&s).map_err(|_| ApiError::internal("could not encode settings"))?;
        torrent::settings_put(State(st.clone()), Bytes::from(body)).await?;
    }
    let mut added = 0;
    for item in v.get("watchlist").and_then(Value::as_array).into_iter().flatten() {
        let item: crate::model::Item =
            serde_json::from_value(item.clone()).map_err(|_| ApiError::bad_request("the watchlist in this file is damaged"))?;
        st.db.put(&item)?;
        added += 1;
    }
    Ok(Json(json!({ "watchlist": added })))
}

fn scan_profile() -> std::path::PathBuf {
    paths::data().join("webview").join("scan")
}

/// `GET /api/maintenance`: sizes of what "Cache leeren" removes, and where the data lives.
async fn maintenance() -> Json<Value> {
    let data = paths::data();
    Json(json!({
        "stream_cache_bytes": paths::dir_size(&data.join("cache")),
        "scan_profile_bytes": paths::dir_size(&scan_profile()),
        "scan_profile_pending": data.join(CLEAR_SCAN_PROFILE).is_file(),
        "data_dir": data.to_string_lossy(),
    }))
}

/// `DELETE /api/maintenance/cache`: stream cache now (files in use stay), saved scan results and the TMDB cache now,
/// the source-search browser profile on the next start (WebView2 holds it open while the app runs).
async fn cache_clear(State(st): St) -> ApiResult<Json<Value>> {
    let data = paths::data();
    let left = paths::clear_dir(&data.join("cache"));
    for table in [Table::Scans, Table::Streams] {
        st.db.blob_prune(table, i64::MAX)?;
    }
    st.tmdb.clear_cache();
    let mut pending = false;
    if scan_profile().is_dir() {
        std::fs::write(data.join(CLEAR_SCAN_PROFILE), b"").map_err(|_| ApiError::internal("cannot mark the profile"))?;
        pending = true;
    }
    Ok(Json(json!({ "stream_cache_bytes": left, "scan_profile_pending": pending })))
}

#[derive(Deserialize)]
struct ResetBody {
    confirm: String,
}

/// Tables a reset empties. `torrents` stays: downloads keep running and their files stay on disk.
const RESET_TABLES: &[&str] = &["watchlist", "progress", "history", "settings", "scans", "streams", "provider_health"];

/// `POST /api/setup/reset {confirm:"reset"}`: back to a fresh install (token, preferences, watchlist, history,
/// progress, settings and the VPN file are gone; downloads and their files stay).
async fn reset(State(st): St, Json(body): Json<ResetBody>) -> ApiResult<StatusCode> {
    if body.confirm != "reset" {
        return Err(ApiError::bad_request("confirm must be 'reset'"));
    }
    // A second connection on the same file: the data layer has no "delete everything" and does not need one.
    let conn = rusqlite::Connection::open(paths::data().join("flimmer.db")).map_err(|_| ApiError::internal("cannot open the database"))?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(|_| ApiError::internal("cannot open the database"))?;
    let sql: String = RESET_TABLES.iter().map(|t| format!("DELETE FROM {t};")).collect();
    conn.execute_batch(&format!("BEGIN;{sql}COMMIT;")).map_err(|_| ApiError::internal("could not reset the data"))?;
    let _ = std::fs::remove_file(paths::vpn_file());
    st.torrents.vpn.down();
    st.tmdb.set_token(String::new());
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_shapes() {
        let jwt = format!("eyJhbGciOiJIUzI1NiJ9.{}.{}", "a".repeat(120), "b-_c".repeat(10));
        assert!(token_shape(&jwt).is_ok());
        assert_eq!(token_shape("0123456789abcdef0123456789ABCDEF").unwrap_err().0, "api_key");
        assert_eq!(token_shape("hello").unwrap_err().0, "format");
        assert_eq!(token_shape(&format!("{jwt}<script>")).unwrap_err().0, "format");
    }

    #[test]
    fn only_fixed_pages_open() {
        assert!(page_url("tmdb_api").is_some_and(|u| u.starts_with("https://www.themoviedb.org/")));
        assert!(page_url("https://evil.example").is_none());
        assert!(page_url("").is_none());
    }

    #[test]
    fn reset_keeps_downloads() {
        assert!(!RESET_TABLES.contains(&"torrents"));
        assert!(RESET_TABLES.contains(&"settings"));
    }
}
