//! Captured streams: proxy with the captured request headers, disk prefetch, cache.

use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWriteExt, task::JoinHandle};
use url::Url;

use crate::{
    lock,
    api::AppState,
    db::Table,
    error::{ApiError, ApiResult},
    hls::{self, ByteRange},
    scan,
};

const CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const CACHE_HEADER: &str = "x-flimmer-cache";
/// Pause between prefetch downloads so the upstream sees no burst.
const POLITE_PAUSE: Duration = Duration::from_millis(60);
/// A playlist is text; anything bigger is not one.
const PLAYLIST_MAX: usize = 8 * 1024 * 1024;
/// One cached segment. The cache is read back into memory, so a "segment" may not be a whole film.
const SEGMENT_MAX: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamKind {
    Hls,
    Mp4,
}

/// A captured stream as stored in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamRow {
    pub id: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub kind: StreamKind,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BufferState {
    pub variant: usize,
    pub from_seconds: f64,
    pub ahead_seconds: f64,
    pub bytes: u64,
    pub done: bool,
}

struct Prefetch {
    task: JoinHandle<()>,
    state: Arc<Mutex<BufferState>>,
}

pub struct Streams {
    pub http: reqwest::Client,
    cache_dir: PathBuf,
    prefetch: Mutex<HashMap<String, Prefetch>>,
}

enum Upstream {
    Expired,
    Status(u16),
    Net(String),
    /// Not an address the proxy may fetch (private network, loopback, no http).
    Blocked,
}

impl From<Upstream> for ApiError {
    fn from(e: Upstream) -> Self {
        match e {
            Upstream::Expired => ApiError::new(StatusCode::GONE, "expired"),
            Upstream::Status(s) => ApiError::bad_gateway(format!("upstream answered HTTP {s}")),
            Upstream::Net(m) => ApiError::bad_gateway(format!("upstream not reachable: {m}")),
            Upstream::Blocked => ApiError::forbidden("this address is not allowed"),
        }
    }
}

fn range_header((off, len): ByteRange) -> String {
    format!("bytes={off}-{}", off + len.max(1) - 1)
}

/// What an upstream may claim to be. Anything a browser would render or run as a page (HTML, SVG, XML, scripts)
/// is handed on as plain bytes, so a hostile stream host cannot put a page into Flimmer's origin.
fn safe_type(upstream: &str) -> Option<&str> {
    let kind = upstream.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    let ok = kind.starts_with("video/")
        || kind.starts_with("audio/")
        || matches!(kind.as_str(), "application/octet-stream" | "application/vnd.apple.mpegurl" | "application/x-mpegurl" | "text/vtt" | "application/mp4" | "image/jpeg" | "image/png" | "image/webp");
    ok.then_some(upstream)
}

fn content_type(url: &str, upstream: Option<&str>) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    let by_ext = match path.rsplit('.').next().unwrap_or("") {
        "ts" => Some("video/mp2t"),
        "m4s" | "mp4" | "m4v" | "cmfv" => Some("video/mp4"),
        "m4a" | "cmfa" => Some("audio/mp4"),
        "aac" => Some("audio/aac"),
        "vtt" | "webvtt" => Some("text/vtt; charset=utf-8"),
        "key" => Some("application/octet-stream"),
        _ => None,
    };
    by_ext.or(upstream.and_then(safe_type)).unwrap_or("application/octet-stream").to_string()
}

impl Streams {
    pub fn new(cache_dir: PathBuf) -> Result<Self, String> {
        // Public addresses only, on every redirect hop (see `guard`).
        let http = crate::guard::outbound_client()?;
        Ok(Self { http, cache_dir, prefetch: Mutex::new(HashMap::new()) })
    }

    /// Deletes cache folders that were not written to for 24 h.
    pub fn housekeeping(&self) {
        let Ok(entries) = std::fs::read_dir(&self.cache_dir) else { return };
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| SystemTime::now().duration_since(t).unwrap_or_default() > CACHE_MAX_AGE)
                .unwrap_or(false);
            if old {
                let path = entry.path();
                let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
            }
        }
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.cache_dir.join(id)
    }

    fn cache_file(&self, id: &str, url: &str, range: Option<&str>) -> PathBuf {
        // DefaultHasher::new() uses fixed keys, so names are stable across restarts.
        let mut h = std::collections::hash_map::DefaultHasher::new();
        url.hash(&mut h);
        range.unwrap_or("").hash(&mut h);
        self.dir(id).join(format!("{:016x}.bin", h.finish()))
    }

    pub fn request(&self, headers: &[(String, String)], url: &str, range: Option<&str>) -> reqwest::RequestBuilder {
        let mut rb = self.http.get(url).header("Accept", "*/*");
        for (k, v) in headers {
            rb = rb.header(k, v);
        }
        if let Some(r) = range {
            rb = rb.header("Range", r);
        }
        rb
    }

    async fn send(&self, row: &StreamRow, url: &str, range: Option<&str>) -> Result<reqwest::Response, Upstream> {
        if !Url::parse(url).is_ok_and(|u| crate::guard::outbound_url_ok(&u)) {
            return Err(Upstream::Blocked);
        }
        let resp = self.request(&row.headers, url, range).send().await.map_err(|e| Upstream::Net(net_msg(&e)))?;
        match resp.status().as_u16() {
            401 | 403 | 410 => Err(Upstream::Expired),
            _ => Ok(resp),
        }
    }

    async fn text(&self, row: &StreamRow, url: &str) -> Result<(Url, String), Upstream> {
        let resp = self.send(row, url, None).await?;
        if !resp.status().is_success() {
            return Err(Upstream::Status(resp.status().as_u16()));
        }
        let base = resp.url().clone();
        let mut resp = resp;
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| Upstream::Net(net_msg(&e)))? {
            if body.len() + chunk.len() > PLAYLIST_MAX {
                return Err(Upstream::Net("the playlist is too large".into()));
            }
            body.extend_from_slice(&chunk);
        }
        Ok((base, String::from_utf8_lossy(&body).into_owned()))
    }

    /// Downloads `url` into the cache unless it is there already. Returns the file size.
    async fn fetch_to_cache(&self, row: &StreamRow, url: &str, range: Option<ByteRange>) -> Result<u64, Upstream> {
        let range = range.map(range_header);
        let dest = self.cache_file(&row.id, url, range.as_deref());
        if let Ok(meta) = tokio::fs::metadata(&dest).await {
            return Ok(meta.len());
        }
        let io = |e: std::io::Error| Upstream::Net(format!("cache write failed: {e}"));
        let mut resp = self.send(row, url, range.as_deref()).await?;
        // A server that ignores Range would hand us the whole file under a range key.
        let wanted = if range.is_some() { resp.status().as_u16() == 206 } else { resp.status().is_success() };
        if !wanted {
            return Err(Upstream::Status(resp.status().as_u16()));
        }
        tokio::fs::create_dir_all(self.dir(&row.id)).await.map_err(io)?;
        let part = dest.with_extension("part");
        let mut file = tokio::fs::File::create(&part).await.map_err(io)?;
        let mut size = 0u64;
        while let Some(chunk) = resp.chunk().await.map_err(|e| Upstream::Net(net_msg(&e)))? {
            file.write_all(&chunk).await.map_err(io)?;
            size += chunk.len() as u64;
            if size > SEGMENT_MAX {
                drop(file);
                let _ = tokio::fs::remove_file(&part).await;
                return Err(Upstream::Net("the segment is too large".into()));
            }
        }
        file.flush().await.map_err(io)?;
        drop(file);
        tokio::fs::rename(&part, &dest).await.map_err(io)?;
        Ok(size)
    }

    fn stop_prefetch(&self, id: &str) {
        if let Some(p) = lock(&self.prefetch).remove(id) {
            p.task.abort();
        }
    }

    /// Stops the prefetch and deletes the cache folder (the stream itself stays usable).
    pub async fn clear(&self, id: &str) {
        self.stop_prefetch(id);
        let _ = tokio::fs::remove_dir_all(self.dir(id)).await;
    }
}

/// reqwest errors can quote the URL (with tokens); keep only the reason.
fn net_msg(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "timeout".into()
    } else if e.is_connect() {
        "connection failed".into()
    } else {
        "transfer failed".into()
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn load(st: &AppState, id: &str) -> ApiResult<StreamRow> {
    if !valid_id(id) {
        return Err(ApiError::not_found("unknown stream"));
    }
    st.db
        .blob_get(Table::Streams, id)?
        .and_then(|data| serde_json::from_str(&data).ok())
        .ok_or_else(|| ApiError::not_found("unknown stream"))
}

type St = State<Arc<AppState>>;
type Params = Query<HashMap<String, String>>;

pub async fn master(State(st): St, Path(id): Path<String>, headers: HeaderMap) -> ApiResult<Response> {
    let row = load(&st, &id)?;
    if row.kind == StreamKind::Mp4 {
        // A plain file has no playlist: `<video src>` follows this to the proxied file.
        let base = Url::parse(&row.url).map_err(|_| ApiError::not_found("unknown stream"))?;
        return Ok(Redirect::temporary(&hls::wrap(&id, &base, &row.url)).into_response());
    }
    let url = row.url.clone();
    proxy(&st, &row, &url, &headers).await
}

pub async fn resource(
    State(st): St,
    Path(id): Path<String>,
    Query(q): Params,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let row = load(&st, &id)?;
    let url = q.get("u").ok_or_else(|| ApiError::bad_request("'u' is required"))?;
    // Only what the server itself wrote into this stream's playlists: the signature covers stream id and URL.
    // So the proxy cannot be pointed at other hosts, and the captured headers go nowhere else.
    if !q.get("s").is_some_and(|sig| crate::guard::stream_signed(&id, url, sig)) {
        return Err(ApiError::forbidden("this address does not belong to the stream"));
    }
    if !Url::parse(url).is_ok_and(|u| crate::guard::outbound_url_ok(&u)) {
        return Err(ApiError::forbidden("this address is not allowed"));
    }
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).map(str::trim);
    if let Some(resp) = from_cache(&st.streams, &id, url, range).await {
        return Ok(resp);
    }
    proxy(&st, &row, url, &headers).await
}

/// `bytes=a-b` / `bytes=a-` against a body of `len` bytes.
fn slice_range(range: &str, len: u64) -> Option<(u64, u64)> {
    let (a, b) = range.strip_prefix("bytes=")?.split_once('-')?;
    let start: u64 = a.trim().parse().ok()?;
    let end = match b.trim() {
        "" => len.checked_sub(1)?,
        b => b.parse::<u64>().ok()?.min(len.checked_sub(1)?),
    };
    (start <= end).then_some((start, end))
}

async fn from_cache(streams: &Streams, id: &str, url: &str, range: Option<&str>) -> Option<Response> {
    let ctype = content_type(url, None);
    let build = |status: StatusCode, body: Vec<u8>, content_range: Option<String>| {
        let mut resp = (status, body).into_response();
        let h = resp.headers_mut();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_str(&ctype).ok()?);
        h.insert(CACHE_HEADER, HeaderValue::from_static("hit"));
        h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
        if let Some(cr) = content_range {
            h.insert(header::CONTENT_RANGE, HeaderValue::from_str(&cr).ok()?);
        }
        Some(resp)
    };
    if let Some(r) = range {
        // exactly this byte range was prefetched (EXT-X-BYTERANGE playlists)
        if let Ok(body) = tokio::fs::read(streams.cache_file(id, url, Some(r))).await {
            let cr = format!("{}/*", r.replacen('=', " ", 1));
            return build(StatusCode::PARTIAL_CONTENT, body, Some(cr));
        }
    }
    let body = tokio::fs::read(streams.cache_file(id, url, None)).await.ok()?;
    match range {
        None => build(StatusCode::OK, body, None),
        Some(r) => {
            let len = body.len() as u64;
            let (a, b) = slice_range(r, len)?;
            let part = body[a as usize..=b as usize].to_vec();
            build(StatusCode::PARTIAL_CONTENT, part, Some(format!("bytes {a}-{b}/{len}")))
        }
    }
}

async fn proxy(st: &AppState, row: &StreamRow, url: &str, req_headers: &HeaderMap) -> ApiResult<Response> {
    let range = req_headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let mut resp = st.streams.send(row, url, range).await?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let base = resp.url().clone();
    let up = |name: header::HeaderName| resp.headers().get(&name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let (ctype, clen, crange) =
        (up(header::CONTENT_TYPE), up(header::CONTENT_LENGTH), up(header::CONTENT_RANGE));
    let encoded = resp.headers().contains_key(header::CONTENT_ENCODING);

    let first = resp.chunk().await.map_err(|e| Upstream::Net(net_msg(&e)))?.unwrap_or_default();
    let head = String::from_utf8_lossy(&first[..first.len().min(64)]).into_owned();
    if status.is_success() && hls::is_playlist(&head) {
        let mut body = first.to_vec();
        while let Some(chunk) = resp.chunk().await.map_err(|e| Upstream::Net(net_msg(&e)))? {
            if body.len() + chunk.len() > PLAYLIST_MAX {
                return Err(ApiError::bad_gateway("the playlist is too large"));
            }
            body.extend_from_slice(&chunk);
        }
        let text = hls::rewrite(&row.id, &base, &String::from_utf8_lossy(&body));
        return Ok((
            [
                (header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            text,
        )
            .into_response());
    }

    let stream = futures_util::stream::unfold((Some(first), Some(resp)), |(first, resp)| async move {
        if let Some(b) = first {
            return Some((Ok::<Bytes, reqwest::Error>(b), (None, resp)));
        }
        let mut resp = resp?;
        match resp.chunk().await {
            Ok(Some(b)) => Some((Ok(b), (None, Some(resp)))),
            Ok(None) => None,
            Err(e) => Some((Err(e), (None, None))),
        }
    });
    let mut out = Response::new(Body::from_stream(stream));
    *out.status_mut() = status;
    let h = out.headers_mut();
    let mut set = |name: header::HeaderName, value: Option<String>| {
        if let Some(v) = value.and_then(|v| HeaderValue::from_str(&v).ok()) {
            h.insert(name, v);
        }
    };
    set(header::CONTENT_TYPE, Some(content_type(url, ctype.as_deref())));
    set(header::CONTENT_RANGE, crange);
    if !encoded {
        set(header::CONTENT_LENGTH, clen);
    }
    set(header::ACCEPT_RANGES, Some("bytes".into()));
    h.insert(CACHE_HEADER, HeaderValue::from_static("miss"));
    Ok(out)
}

#[derive(Deserialize)]
pub struct PrefetchParams {
    #[serde(default)]
    variant: usize,
    #[serde(default)]
    from: f64,
}

pub async fn prefetch(
    State(st): St,
    Path(id): Path<String>,
    Query(p): Query<PrefetchParams>,
) -> ApiResult<StatusCode> {
    let row = load(&st, &id)?;
    if row.kind != StreamKind::Hls {
        return Err(ApiError::bad_request("prefetch needs an HLS stream"));
    }
    if !p.from.is_finite() || p.from < 0.0 {
        return Err(ApiError::bad_request("'from' must be seconds >= 0"));
    }
    let settings = scan::load_settings(&st)?;

    // Resolve the playlists first so a bad index or an expired stream is answered here.
    let (base, body) = st.streams.text(&row, &row.url).await?;
    let mut playlists = Vec::with_capacity(2);
    match hls::parse_master(&body) {
        Some(master) => {
            let variant =
                master.variants.get(p.variant).ok_or_else(|| ApiError::bad_request("'variant' is out of range"))?;
            // the video, plus its audio when that is a playlist of its own
            let audio = master.audio_uri(variant, &settings.filter.audio_lang);
            for uri in std::iter::once(variant.uri.as_str()).chain(audio) {
                let url = base.join(uri).map_err(|_| ApiError::bad_gateway("playlist URI is not a URL"))?;
                playlists.push(st.streams.text(&row, url.as_str()).await?);
            }
        }
        None if p.variant == 0 => playlists.push((base, body)),
        None => return Err(ApiError::bad_request("'variant' is out of range")),
    }

    st.streams.stop_prefetch(&id);
    let state = Arc::new(Mutex::new(BufferState { variant: p.variant, from_seconds: p.from, ..Default::default() }));
    let until = p.from + f64::from(settings.prefetch_minutes) * 60.0;
    let task = tokio::spawn(run_prefetch(st.clone(), row, playlists, p.from, until, state.clone()));
    lock(&st.streams.prefetch).insert(id, Prefetch { task, state });
    Ok(StatusCode::NO_CONTENT)
}

/// One file a segment needs: (absolute URL, byte range).
type Part = (String, Option<ByteRange>);

async fn run_prefetch(
    st: Arc<AppState>,
    row: StreamRow,
    playlists: Vec<(Url, String)>,
    from: f64,
    until: f64,
    state: Arc<Mutex<BufferState>>,
) {
    let streams = &st.streams;
    // per playlist: the segments of the window, each with the key / init file it needs
    let mut tracks: Vec<Vec<(f64, Vec<Part>)>> = Vec::with_capacity(playlists.len());
    for (base, body) in &playlists {
        let abs = |uri: &str| base.join(uri).map(String::from).ok();
        let mut track = Vec::new();
        for seg in hls::parse_media(body) {
            let end = seg.start + seg.duration;
            if end <= from {
                continue;
            }
            if seg.start >= until {
                break;
            }
            let mut parts: Vec<Part> = Vec::with_capacity(3);
            parts.extend(seg.key.as_deref().and_then(abs).map(|u| (u, None)));
            parts.extend(seg.map.as_ref().and_then(|(u, r)| Some((abs(u)?, *r))));
            let Some(url) = abs(&seg.uri) else { return };
            parts.push((url, seg.range));
            track.push((end, parts));
        }
        tracks.push(track);
    }

    // Walk all playlists in step so video and audio are buffered equally far.
    let mut counted: HashSet<Part> = HashSet::new();
    for i in 0..tracks.iter().map(Vec::len).max().unwrap_or(0) {
        let mut bytes = 0;
        let mut ahead = f64::INFINITY;
        for track in &tracks {
            let Some((end, parts)) = track.get(i) else { continue };
            for part in parts {
                let (url, range) = part;
                let mut res = streams.fetch_to_cache(&row, url, *range).await;
                if matches!(res, Err(Upstream::Net(_) | Upstream::Status(500..))) {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    res = streams.fetch_to_cache(&row, url, *range).await;
                }
                match res {
                    Ok(n) if counted.insert(part.clone()) => bytes += n,
                    Ok(_) => {}
                    // Expired or broken upstream: stop here, `done` stays false.
                    Err(_) => return,
                }
            }
            ahead = ahead.min(end - from);
        }
        {
            let mut s = lock(&state);
            if ahead.is_finite() {
                s.ahead_seconds = ahead.max(0.0);
            }
            s.bytes += bytes;
        }
        tokio::time::sleep(POLITE_PAUSE).await;
    }
    lock(&state).done = true;
}

pub async fn buffer(State(st): St, Path(id): Path<String>) -> ApiResult<Json<BufferState>> {
    load(&st, &id)?;
    let state = lock(&st.streams.prefetch).get(&id).map(|p| lock(&p.state).clone());
    Ok(Json(state.unwrap_or_default()))
}

pub async fn delete(State(st): St, Path(id): Path<String>) -> ApiResult<StatusCode> {
    if !valid_id(&id) {
        return Err(ApiError::not_found("unknown stream"));
    }
    st.streams.clear(&id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a stream completely (replaced by a re-scan).
pub async fn forget(st: &AppState, id: &str) {
    if valid_id(id) {
        st.streams.clear(id).await;
        let _ = st.db.blob_delete(Table::Streams, id);
    }
}
