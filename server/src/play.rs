//! Playback of torrent files. Browser-playable files (MP4/WebM with H.264/HEVC/VP9/AV1 + AAC/MP3/Opus)
//! are served directly with HTTP range support (native seeking; the torrent engine prioritises the
//! requested position). Everything else (MKV, AC3/DTS/EAC3 audio …) goes through ffmpeg on the fly as
//! fragmented MP4: video is copied when the browser can decode it, audio becomes AAC stereo.
//!
//! Streaming order of pieces: the engine fetches 32 MiB from every open reader's position first.
//! On top of that, [`prime`] opens readers at a file's start and end when it is first wanted
//! (container headers and index), and [`window`] keeps one reader at the end of what is buffered,
//! so the readahead grows with the bitrate. A seek is a new request: its reader moves the
//! priority to the new position at once.

use std::{
    io::SeekFrom,
    path::PathBuf,
    pin::Pin,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use librqbit::ManagedTorrent;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, ReadBuf},
    process::Command,
    sync::OnceCell,
};
use tokio_util::io::ReaderStream;

use crate::{
    api::{AppState, Params, St},
    error::{ApiError, ApiResult},
    torrent::{Entry, Meta, Torrents, is_video},
};

const COPY_VIDEO: &[&str] = &["h264", "hevc", "vp8", "vp9", "av1"];
const DIRECT_AUDIO: &[&str] = &["aac", "mp3", "opus", "vorbis", "flac"];
const DIRECT_EXT: &[&str] = &["mp4", "m4v", "mov", "webm"];
/// Subtitle codecs ffmpeg can turn into WebVTT (bitmap subtitles like PGS/VobSub cannot).
const TEXT_SUBS: &[&str] = &["subrip", "srt", "ass", "ssa", "webvtt", "mov_text", "text"];
const MIB: u64 = 1024 * 1024;
/// What the engine reads ahead of every open reader on its own.
const ENGINE_WINDOW: u64 = 32 * MIB;
/// Bytes at a file's start and end that are fetched first.
const HEAD: u64 = 6 * MIB;
const TAIL: u64 = 4 * MIB;
/// Seconds of playback the readahead window aims for.
const WINDOW_SECONDS: u64 = 120;
const WINDOW_MAX: u64 = 512 * MIB;
/// A file counts as "being played" this long after its last read.
const PLAYING_FOR: Duration = Duration::from_secs(20);

trait ReadSeek: AsyncRead + AsyncSeek + Send + Unpin {}
impl<T: AsyncRead + AsyncSeek + Send + Unpin> ReadSeek for T {}

#[derive(Debug, Clone, Serialize)]
pub struct AudioTrack {
    /// Position among the audio streams (the `audio` parameter of `/play`).
    pub index: usize,
    pub codec: String,
    pub channels: u32,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub default: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubtitleTrack {
    /// Position among the subtitle streams (the `track` parameter of `/subtitle`).
    pub index: usize,
    pub codec: String,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub default: bool,
    pub forced: bool,
    /// Can be served as WebVTT (text based); bitmap subtitles cannot.
    pub text: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub duration: Option<f64>,
    pub video_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub audio: Vec<AudioTrack>,
    pub subtitles: Vec<SubtitleTrack>,
    /// Plays as it is in the browser.
    pub direct: bool,
    /// ffmpeg would copy the video stream (no re-encode).
    pub copy_video: bool,
}

fn tool(st: &AppState, name: &str) -> Option<PathBuf> {
    let dir = st.torrents.root.join("tools").join("ffmpeg");
    [dir.join("bin").join(name), dir.join(name)].into_iter().find(|p| p.is_file())
}

fn command(path: PathBuf) -> Command {
    let mut cmd = Command::new(path);
    cmd.stdin(Stdio::null()).stderr(Stdio::null()).stdout(Stdio::piped()).kill_on_drop(true);
    // No console window flashing up under the desktop app.
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd
}

fn extension(path: &str) -> String {
    path.rsplit('.').next().unwrap_or_default().to_ascii_lowercase()
}

/// The file as ffmpeg/ffprobe read it: Flimmer's own range endpoint.
fn file_url(st: &AppState, id: &str, index: usize) -> String {
    format!("http://127.0.0.1:{}/api/torrents/{id}/file/{index}", st.torrents.port)
}

pub fn probe_from(json: &Value, ext: &str) -> Probe {
    let streams = json.get("streams").and_then(Value::as_array).cloned().unwrap_or_default();
    let kind = |s: &Value, k: &str| s.get("codec_type").and_then(Value::as_str) == Some(k);
    let text = |s: &Value, k: &str| s.pointer(k).and_then(Value::as_str).map(str::to_owned);
    // Cover art is a "video" stream too.
    let video = streams.iter().find(|s| kind(s, "video") && s.pointer("/disposition/attached_pic").and_then(Value::as_u64) != Some(1));
    let audio: Vec<AudioTrack> = streams
        .iter()
        .filter(|s| kind(s, "audio"))
        .enumerate()
        .map(|(index, s)| AudioTrack {
            index,
            codec: text(s, "/codec_name").unwrap_or_default(),
            channels: s.get("channels").and_then(Value::as_u64).unwrap_or(0) as u32,
            lang: text(s, "/tags/language"),
            title: text(s, "/tags/title"),
            default: s.pointer("/disposition/default").and_then(Value::as_u64) == Some(1),
        })
        .collect();
    let flag = |s: &Value, k: &str| s.pointer(k).and_then(Value::as_u64) == Some(1);
    let subtitles: Vec<SubtitleTrack> = streams
        .iter()
        .filter(|s| kind(s, "subtitle"))
        .enumerate()
        .map(|(index, s)| {
            let codec = text(s, "/codec_name").unwrap_or_default();
            SubtitleTrack {
                index,
                text: TEXT_SUBS.contains(&codec.as_str()),
                codec,
                lang: text(s, "/tags/language"),
                title: text(s, "/tags/title"),
                default: flag(s, "/disposition/default"),
                forced: flag(s, "/disposition/forced"),
            }
        })
        .collect();
    let video_codec = video.and_then(|v| text(v, "/codec_name"));
    let copy_video = video_codec.as_deref().is_some_and(|c| COPY_VIDEO.contains(&c));
    let dim = |k: &str| video.and_then(|v| v.get(k)).and_then(Value::as_u64).map(|n| n as u32);
    Probe {
        duration: json.pointer("/format/duration").and_then(Value::as_str).and_then(|d| d.parse().ok()),
        direct: DIRECT_EXT.contains(&ext) && copy_video && audio.first().is_none_or(|a| DIRECT_AUDIO.contains(&a.codec.as_str())),
        copy_video,
        width: dim("width"),
        height: dim("height"),
        video_codec,
        audio,
        subtitles,
    }
}

/// ISO 639-2 codes (as ffprobe reports them) of a two-letter language.
fn lang_matches(track: Option<&str>, wanted: &str) -> bool {
    let Some(t) = track.map(str::to_ascii_lowercase) else { return false };
    match wanted {
        "de" => matches!(t.as_str(), "de" | "ger" | "deu" | "german" | "deutsch"),
        "en" => matches!(t.as_str(), "en" | "eng" | "english"),
        w => t == w,
    }
}

/// The audio track to play when the player names none: the saved language wish, else the file's default.
pub fn default_audio(audio: &[AudioTrack], wanted_lang: &str) -> usize {
    audio
        .iter()
        .find(|a| lang_matches(a.lang.as_deref(), wanted_lang))
        .or_else(|| audio.iter().find(|a| a.default))
        .map_or(0, |a| a.index)
}

// ---------- read position, readahead window, stream health ----------

/// One file that is being read (by the player directly or by ffmpeg).
pub struct Playback {
    /// Position of the newest reader, bytes into the file.
    pos: AtomicU64,
    touched: Mutex<Instant>,
    /// Bytes per second of playback (file size / duration); 0 = not known yet.
    bitrate: AtomicU64,
    /// A window task is running for it.
    watched: AtomicBool,
}

impl Playback {
    fn touch(&self, pos: u64) {
        self.pos.store(pos, Ordering::Relaxed);
        *self.touched.lock().unwrap() = Instant::now();
    }

    fn playing(&self) -> bool {
        self.touched.lock().unwrap().elapsed() < PLAYING_FOR
    }
}

/// A reader that reports where it is, so the window follows it.
struct Tracked<R> {
    inner: R,
    play: Arc<Playback>,
    pos: u64,
}

impl<R: AsyncRead + Unpin> AsyncRead for Tracked<R> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        let read = (buf.filled().len() - before) as u64;
        if read > 0 {
            self.pos += read;
            self.play.touch(self.pos);
        }
        poll
    }
}

/// Bytes available without a gap from `pos` on: the run of present pieces that starts at the
/// piece holding `pos`. `bits` = one bit per piece, most significant bit first.
pub fn buffered_ahead(bits: &[u8], piece_len: u64, file_offset: u64, file_len: u64, pos: u64) -> u64 {
    if piece_len == 0 || pos >= file_len {
        return 0;
    }
    let have = |i: u64| bits.get((i / 8) as usize).is_some_and(|b| b & (0x80 >> (i % 8)) != 0);
    let (start, end) = (file_offset + pos, file_offset + file_len);
    let mut piece = start / piece_len;
    while piece * piece_len < end && have(piece) {
        piece += 1;
    }
    (piece * piece_len).min(end).saturating_sub(start)
}

/// How far ahead of the play position to fetch: two minutes of playback, at least the engine's own window.
pub fn window_bytes(bitrate: u64, file_len: u64) -> u64 {
    let wanted = if bitrate == 0 { ENGINE_WINDOW * 2 } else { bitrate * WINDOW_SECONDS };
    wanted.clamp(ENGINE_WINDOW, WINDOW_MAX).min(file_len)
}

fn playback(tor: &Torrents, id: &str, file: usize) -> Arc<Playback> {
    tor.playing
        .lock()
        .unwrap()
        .entry((id.to_owned(), file))
        .or_insert_with(|| {
            Arc::new(Playback { pos: AtomicU64::new(0), touched: Mutex::new(Instant::now()), bitrate: AtomicU64::new(0), watched: AtomicBool::new(false) })
        })
        .clone()
}

/// Fetches a file's first and last megabytes ahead of everything else: the container header at the
/// start, and what players and ffprobe look for at the end (MP4 `moov`, MKV cues).
pub fn prime(st: &Arc<AppState>, e: &Arc<Entry>, file: usize) {
    let (st, e) = (st.clone(), e.clone());
    tokio::spawn(async move {
        let Some(handle) = e.handle() else { return };
        let Ok(mut head) = handle.clone().stream(file).await else { return };
        let len = head.len();
        let Ok(mut tail) = handle.clone().stream(file).await else { return };
        let tail_at = len.saturating_sub(TAIL);
        if tail.seek(SeekFrom::Start(tail_at)).await.is_err() {
            return;
        }
        // The open readers are the priority; hold them until both ends are there.
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(120) && st.torrents.has(&e.id) && !e.meta().paused {
            let Some((bits, piece, offset, flen)) = st.torrents.haves(&handle, file) else { break };
            let head_done = buffered_ahead(&bits, piece, offset, flen, 0) >= HEAD.min(flen);
            let tail_done = buffered_ahead(&bits, piece, offset, flen, tail_at) >= flen - tail_at;
            if head_done && tail_done {
                break;
            }
            // Keep the head reader at the first missing byte so its window stays on the start.
            if head_done {
                let _ = head.seek(SeekFrom::Start(0)).await;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
}

/// Keeps the readahead window of one playing file filled: a helper reader sits at the end of what
/// is buffered whenever the engine's own 32 MiB are (nearly) there and the window wants more.
fn window(st: Arc<AppState>, e: Arc<Entry>, file: usize, play: Arc<Playback>) {
    if play.watched.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        let mut helper = None;
        while play.playing() && st.torrents.has(&e.id) {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let Some(handle) = e.handle() else { break };
            let Some((bits, piece, offset, len)) = st.torrents.haves(&handle, file) else { break };
            let pos = play.pos.load(Ordering::Relaxed).min(len);
            let ahead = buffered_ahead(&bits, piece, offset, len, pos);
            let want = window_bytes(play.bitrate.load(Ordering::Relaxed), len);
            let edge = pos + ahead;
            if ahead >= ENGINE_WINDOW * 3 / 4 && ahead < want && edge < len {
                if helper.is_none() {
                    helper = handle.clone().stream(file).await.ok();
                }
                if let Some(h) = helper.as_mut() {
                    let _ = h.seek(SeekFrom::Start(edge)).await;
                }
            } else {
                helper = None;
            }
        }
        play.watched.store(false, Ordering::SeqCst);
    });
}

/// `Torrent.stream_health`: how well the file that plays (or would play) is buffered.
pub fn stream_health(tor: &Torrents, e: &Entry, m: &Meta, handle: Option<&Arc<ManagedTorrent>>, down_speed: u64, peers: u64) -> Value {
    let playing = tor
        .playing
        .lock()
        .unwrap()
        .iter()
        .filter(|((id, _), p)| *id == e.id && p.playing())
        .max_by_key(|(_, p)| *p.touched.lock().unwrap())
        .map(|((_, file), p)| (*file, p.clone()));
    let first_video = m.selected.iter().copied().find(|i| m.files.iter().any(|f| f.index == *i && is_video(&f.path)));
    let Some(file) = playing.as_ref().map(|p| p.0).or(first_video).or(m.selected.first().copied()) else { return Value::Null };
    let Some(size) = m.files.iter().find(|f| f.index == file).map(|f| f.size) else { return Value::Null };
    let position = playing.as_ref().map_or(0, |p| p.1.pos.load(Ordering::Relaxed)).min(size);
    let duration = tor.probes.lock().unwrap().get(&(e.id.clone(), file)).and_then(|p| p.duration).filter(|d| *d > 0.0);
    let bitrate = duration.map(|d| (size as f64 / d) as u64).filter(|b| *b > 0);
    if let (Some(p), Some(b)) = (&playing, bitrate) {
        p.1.bitrate.store(b, Ordering::Relaxed);
    }
    let buffered = if m.completed {
        size - position
    } else {
        handle.and_then(|h| tor.haves(h, file)).map_or(0, |(bits, piece, offset, len)| buffered_ahead(&bits, piece, offset, len, position))
    };
    json!({
        "file": file, "playing": playing.is_some(),
        "position_bytes": position, "buffered_bytes": buffered,
        "buffered_seconds": bitrate.map(|b| (buffered as f64 / b as f64 * 10.0).round() / 10.0),
        "position_seconds": bitrate.map(|b| (position as f64 / b as f64 * 10.0).round() / 10.0),
        "bitrate": bitrate.map(|b| b * 8), "window_bytes": window_bytes(bitrate.unwrap_or(0), size),
        "peers": peers, "down_speed": down_speed,
    })
}

// ---------- H.264 encoder ----------

/// The H.264 encoder for re-encodes: AMD's hardware encoder when this PC has it, else x264.
async fn h264_encoder(st: &AppState) -> &'static str {
    static ENCODER: OnceCell<&'static str> = OnceCell::const_new();
    ENCODER
        .get_or_init(|| async {
            let Some(ffmpeg) = tool(st, "ffmpeg.exe") else { return "libx264" };
            // Encodes a few black frames: only succeeds with a working AMF runtime and GPU.
            let mut cmd = command(ffmpeg);
            cmd.args(["-v", "error", "-f", "lavfi", "-i", "color=black:s=640x360:r=25:d=0.4", "-c:v", "h264_amf", "-f", "null", "-"]);
            cmd.stdout(Stdio::null());
            match tokio::time::timeout(Duration::from_secs(15), cmd.status()).await {
                Ok(Ok(status)) if status.success() => "h264_amf",
                _ => "libx264",
            }
        })
        .await
}

/// ffmpeg arguments that encode the video stream as 8-bit H.264 (HEVC 10-bit sources included).
pub fn h264_args(encoder: &str) -> Vec<&'static str> {
    match encoder {
        "h264_amf" => vec![
            "-c:v", "h264_amf", "-usage", "transcoding", "-quality", "speed", "-rc", "vbr_peak", "-b:v", "10M", "-maxrate", "16M",
            "-bufsize", "20M", "-g", "60", "-pix_fmt", "nv12",
        ],
        _ => vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", "22", "-pix_fmt", "yuv420p"],
    }
}

async fn probe(st: &Arc<AppState>, e: &Entry, index: usize, path: &str) -> ApiResult<Arc<Probe>> {
    let key = (e.id.clone(), index);
    if let Some(p) = st.torrents.probes.lock().unwrap().get(&key) {
        return Ok(p.clone());
    }
    let ffprobe = tool(st, "ffprobe.exe").ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "ffprobe is missing (tools/ffmpeg)"))?;
    let run = command(ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(file_url(st, &e.id, index))
        .output();
    let out = tokio::time::timeout(Duration::from_secs(90), run)
        .await
        .map_err(|_| ApiError::new(StatusCode::GATEWAY_TIMEOUT, "the file's start is not downloaded yet"))?
        .map_err(|_| ApiError::internal("cannot run ffprobe"))?;
    let json: Value = serde_json::from_slice(&out.stdout).map_err(|_| ApiError::bad_gateway("ffprobe could not read the file"))?;
    let p = Arc::new(probe_from(&json, &extension(path)));
    if p.video_codec.is_none() && p.audio.is_empty() {
        return Err(ApiError::bad_gateway("ffprobe found no playable stream"));
    }
    st.torrents.probes.lock().unwrap().insert(key, p.clone());
    Ok(p)
}

/// The file to play: `?file=`, else the first selected video.
fn pick_file(e: &Entry, q: &std::collections::HashMap<String, String>) -> ApiResult<(usize, String, u64)> {
    let m = e.meta();
    let index = match q.get("file") {
        Some(v) => v.parse().map_err(|_| ApiError::bad_request("'file' must be a file index"))?,
        None => m
            .selected
            .iter()
            .copied()
            .find(|i| m.files.iter().any(|f| f.index == *i && is_video(&f.path)))
            .or(m.selected.first().copied())
            .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "torrent metadata is not resolved yet"))?,
    };
    let f = m.files.iter().find(|f| f.index == index).ok_or_else(|| ApiError::not_found("unknown file"))?;
    if !m.selected.contains(&index) {
        return Err(ApiError::new(StatusCode::CONFLICT, "file is not selected for download"));
    }
    Ok((index, f.path.clone(), f.size))
}

/// Finished downloads come from disk (works offline and after a restart), running ones from the engine.
async fn open(e: &Entry, index: usize, path: &str, size: u64) -> ApiResult<Box<dyn ReadSeek>> {
    let m = e.meta();
    let on_disk = PathBuf::from(&m.folder).join(path);
    let complete = m.completed
        || e.handle().is_some_and(|h| h.stats().file_progress.get(index).is_some_and(|done| *done >= size));
    if complete {
        let file = tokio::fs::File::open(&on_disk).await.map_err(|_| ApiError::not_found("file is missing on disk"))?;
        return Ok(Box::new(file));
    }
    if m.paused {
        return Err(ApiError::new(StatusCode::CONFLICT, "torrent is paused"));
    }
    let handle = e.handle().ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "torrent is not started yet"))?;
    let stream = handle.stream(index).await.map_err(|err| ApiError::internal(format!("cannot open stream: {err:#}")))?;
    Ok(Box::new(stream))
}

fn content_type(path: &str) -> &'static str {
    match extension(path).as_str() {
        "mp4" | "m4v" | "mov" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "ts" | "m2ts" => "video/mp2t",
        _ => "application/octet-stream",
    }
}

/// `bytes=a-b`, `bytes=a-`, `bytes=-n` → inclusive (start, end).
pub fn parse_range(header: &str, size: u64) -> Option<(u64, u64)> {
    let (a, b) = header.strip_prefix("bytes=")?.split(',').next()?.trim().split_once('-')?;
    let last = size.checked_sub(1)?;
    let (start, end) = match (a.is_empty(), b.is_empty()) {
        (true, false) => (size.saturating_sub(b.parse().ok()?), last),
        (false, true) => (a.parse().ok()?, last),
        (false, false) => (a.parse().ok()?, b.parse::<u64>().ok()?.min(last)),
        (true, true) => return None,
    };
    (start <= end && start <= last).then_some((start, end))
}

async fn serve(st: &Arc<AppState>, e: &Arc<Entry>, index: usize, path: &str, size: u64, headers: &HeaderMap) -> ApiResult<Response> {
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let (start, end, status) = match range {
        Some(r) => match parse_range(r, size) {
            Some((a, b)) => (a, b, StatusCode::PARTIAL_CONTENT),
            None => {
                return Ok((StatusCode::RANGE_NOT_SATISFIABLE, [(header::CONTENT_RANGE, format!("bytes */{size}"))]).into_response());
            }
        },
        None => (0, size.saturating_sub(1), StatusCode::OK),
    };
    let mut reader = open(e, index, path, size).await?;
    // The reader's position is what the engine fetches first: a seek re-prioritises here.
    reader.seek(SeekFrom::Start(start)).await.map_err(|_| ApiError::internal("seek failed"))?;
    let len = if size == 0 { 0 } else { end - start + 1 };
    let play = playback(&st.torrents, &e.id, index);
    // Tiny reads at the end of the file (index lookups of players and ffmpeg) do not move the window.
    let probe_read = len < MIB && start + len >= size && start > 0;
    if !probe_read {
        play.touch(start);
    }
    if !e.meta.lock().unwrap().completed {
        window(st.clone(), e.clone(), index, play.clone());
    }
    let reader: Box<dyn AsyncRead + Send + Unpin> = if probe_read { Box::new(reader) } else { Box::new(Tracked { inner: reader, play, pos: start }) };
    let body = Body::from_stream(ReaderStream::with_capacity(reader.take(len), 256 * 1024));
    let mut resp = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type(path))
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, len);
    if status == StatusCode::PARTIAL_CONTENT {
        resp = resp.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"));
    }
    resp.body(body).map_err(|_| ApiError::internal("response"))
}

/// `GET /api/torrents/:id/file/:index`: the raw file with range support (also what ffmpeg reads).
pub async fn file(State(st): St, Path((id, index)): Path<(String, usize)>, headers: HeaderMap) -> ApiResult<Response> {
    let e = st.torrents.entry(&id)?;
    let (index, path, size) = pick_file(&e, &[("file".to_owned(), index.to_string())].into())?;
    serve(&st, &e, index, &path, size, &headers).await
}

/// `GET /api/torrents/:id/playinfo?file=`
pub async fn playinfo(State(st): St, Path(id): Path<String>, Query(q): Params) -> ApiResult<Json<Value>> {
    let e = st.torrents.entry(&id)?;
    let (index, path, size) = pick_file(&e, &q)?;
    let p = probe(&st, &e, index, &path).await?;
    let wanted = crate::scan::load_settings(&st)?.filter.audio_lang;
    let m = e.meta();
    let next = crate::torrent::next_episode(&m.files, m.media.as_ref().and_then(|x| x.season), index);
    Ok(Json(serde_json::json!({
        "file": index, "name": path, "size": size,
        "mode": if p.direct { "direct" } else { "transcode" },
        "url": format!("/api/torrents/{id}/play?file={index}"),
        "duration": p.duration, "video_codec": p.video_codec, "width": p.width, "height": p.height,
        "copy_video": p.copy_video, "audio": p.audio,
        "default_audio": default_audio(&p.audio, &wanted),
        "subtitles": p.subtitles,
        "hevc": p.video_codec.as_deref() == Some("hevc"),
        "h264_encoder": h264_encoder(&st).await,
        "next_file": next.map(|(season, episode, file)| serde_json::json!({ "file": file, "season": season, "episode": episode })),
    })))
}

/// `GET /api/torrents/:id/subtitle?file=&track=&t=`: one subtitle stream of the file as WebVTT.
pub async fn subtitle(State(st): St, Path(id): Path<String>, Query(q): Params) -> ApiResult<Response> {
    let e = st.torrents.entry(&id)?;
    let (index, path, _) = pick_file(&e, &q)?;
    let track: usize = q.get("track").map_or(Ok(0), |v| v.parse()).map_err(|_| ApiError::bad_request("'track' must be a subtitle index"))?;
    let t: f64 = q.get("t").map_or(Ok(0.0), |v| v.parse()).map_err(|_| ApiError::bad_request("'t' must be a number"))?;
    let p = probe(&st, &e, index, &path).await?;
    let sub = p.subtitles.get(track).ok_or_else(|| ApiError::not_found("unknown subtitle track"))?;
    if !sub.text {
        return Err(ApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, format!("'{}' subtitles are images and cannot be shown as text", sub.codec)));
    }
    let ffmpeg = tool(&st, "ffmpeg.exe").ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "ffmpeg is missing (tools/ffmpeg)"))?;
    let mut cmd = command(ffmpeg);
    cmd.args(["-v", "error", "-nostdin"]);
    // No `-ss`: after a seek ffmpeg shifts the cue times by its seek margin.
    // The cues keep the file's own times; `VttShift` moves them to the stream's zero (`t`) exactly.
    cmd.arg("-copyts").arg("-i").arg(file_url(&st, &id, index));
    cmd.args(["-map", &format!("0:s:{track}"), "-vn", "-an", "-c:s", "webvtt", "-f", "webvtt", "pipe:1"]);
    let mut child = cmd.spawn().map_err(|_| ApiError::internal("cannot run ffmpeg"))?;
    let stdout = child.stdout.take().ok_or_else(|| ApiError::internal("cannot run ffmpeg"))?;
    let lines = tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(stdout));
    let stream = futures_util::stream::unfold((lines, child, VttShift::new(t.max(0.0))), |(mut lines, child, mut shift)| async move {
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if let Some(out) = shift.line(line.trim_end_matches('\r')) {
                        return Some((Ok::<_, std::io::Error>(axum::body::Bytes::from(out + "\n")), (lines, child, shift)));
                    }
                }
                Ok(None) => return None,
                Err(e) => return Some((Err(e), (lines, child, shift))),
            }
        }
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "text/vtt; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .map_err(|_| ApiError::internal("response"))
}

/// ffmpeg seeks this much before the asked time when the video has reordered (B) frames
/// (3/23 s, its DTS heuristic). To start on a keyframe, `/play` is asked for a little more.
const SEEK_MARGIN: f64 = 0.14;

/// The time of the last keyframe at or before `t`, from ffprobe's `pts_time` lines.
pub fn snap_to_keyframe(pts_lines: &str, t: f64) -> Option<f64> {
    pts_lines
        .lines()
        .filter_map(|l| l.trim().trim_end_matches(',').parse::<f64>().ok())
        .filter(|pts| *pts <= t + 0.001)
        .fold(None, |best: Option<f64>, pts| Some(best.map_or(pts, |b| b.max(pts))))
}

fn vtt_time(text: &str) -> Option<f64> {
    let mut seconds = 0.0;
    for part in text.trim().split(':') {
        seconds = seconds * 60.0 + part.parse::<f64>().ok().filter(|v| *v >= 0.0)?;
    }
    Some(seconds)
}

fn vtt_stamp(seconds: f64) -> String {
    let ms = (seconds * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Moves the cues of a WebVTT stream `offset` seconds earlier, line by line: cues that are over
/// by then are dropped, one that is on screen at `offset` starts at 0.
pub struct VttShift {
    offset: f64,
    dropping: bool,
}

impl VttShift {
    pub fn new(offset: f64) -> Self {
        Self { offset, dropping: false }
    }

    /// The line to send on, or `None` when it belongs to a dropped cue.
    pub fn line(&mut self, line: &str) -> Option<String> {
        if let Some((from, rest)) = line.split_once("-->") {
            let (to, settings) = rest.trim_start().split_once(' ').unwrap_or((rest.trim(), ""));
            if let (Some(a), Some(b)) = (vtt_time(from), vtt_time(to)) {
                self.dropping = b - self.offset <= 0.0;
                return (!self.dropping).then(|| {
                    let tail = if settings.trim().is_empty() { String::new() } else { format!(" {}", settings.trim()) };
                    format!("{} --> {}{tail}", vtt_stamp((a - self.offset).max(0.0)), vtt_stamp(b - self.offset))
                });
            }
        }
        if line.trim().is_empty() {
            // A blank line ends a cue.
            let was_dropping = std::mem::replace(&mut self.dropping, false);
            return (!was_dropping).then(String::new);
        }
        (!self.dropping).then(|| line.to_owned())
    }
}

/// `GET /api/torrents/:id/seek?file=&t=&video=h264&hevc=0`: how to start the stream at `t`. A copied
/// video stream can only start at a keyframe, so it begins a little earlier than asked. Returns
/// `t` (what to pass to `/play`) and `start` (the file time of the stream's first frame: the
/// player's timeline offset, and the `t` of `/subtitle`).
pub async fn seek(State(st): St, Path(id): Path<String>, Query(q): Params) -> ApiResult<Json<Value>> {
    let e = st.torrents.entry(&id)?;
    let (index, path, _) = pick_file(&e, &q)?;
    let t: f64 = q.get("t").map_or(Ok(0.0), |v| v.parse()).map_err(|_| ApiError::bad_request("'t' must be a number"))?;
    let t = t.max(0.0);
    let p = probe(&st, &e, index, &path).await?;
    let no_hevc = q.get("hevc").is_some_and(|v| matches!(v.as_str(), "0" | "false"));
    let reencode = !p.copy_video || q.get("video").is_some_and(|v| v == "h264") || (no_hevc && p.video_codec.as_deref() == Some("hevc"));
    // A re-encoded stream starts exactly where asked.
    if reencode || t == 0.0 {
        return Ok(Json(json!({ "t": t, "start": t, "requested": t, "exact": true })));
    }
    let ffprobe = tool(&st, "ffprobe.exe").ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "ffprobe is missing (tools/ffmpeg)"))?;
    let run = command(ffprobe)
        .args(["-v", "error", "-select_streams", "v:0", "-skip_frame", "nokey", "-show_entries", "frame=pts_time", "-of", "csv=p=0"])
        .args(["-read_intervals", &format!("{:.3}%{:.3}", (t - 30.0).max(0.0), t + 0.01)])
        .arg(file_url(&st, &id, index))
        .output();
    let out = tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .map_err(|_| ApiError::new(StatusCode::GATEWAY_TIMEOUT, "this part of the file is not downloaded yet"))?
        .map_err(|_| ApiError::internal("cannot run ffprobe"))?;
    let ms = |v: f64| (v * 1000.0).round() / 1000.0;
    Ok(Json(match snap_to_keyframe(&String::from_utf8_lossy(&out.stdout), t) {
        Some(key) => json!({ "t": ms(key + SEEK_MARGIN), "start": ms(key), "requested": t, "exact": (key - t).abs() < 0.002 }),
        // No keyframe found in the half minute before `t`: play from where it was asked.
        None => json!({ "t": t, "start": t, "requested": t, "exact": false }),
    }))
}

/// `GET /api/torrents/:id/play?file=&t=&audio=&video=h264&hevc=0`
pub async fn play(State(st): St, Path(id): Path<String>, Query(q): Params, headers: HeaderMap) -> ApiResult<Response> {
    let e = st.torrents.entry(&id)?;
    let (index, path, size) = pick_file(&e, &q)?;
    let number = |k: &str| q.get(k).map(|v| v.parse::<f64>().map_err(|_| ApiError::bad_request(format!("'{k}' must be a number")))).transpose();
    let t = number("t")?.unwrap_or(0.0).max(0.0);
    let audio = number("audio")?.map(|a| a as usize);
    let p = probe(&st, &e, index, &path).await?;
    // `hevc=0`: the player cannot decode HEVC, so that codec is re-encoded.
    let no_hevc = q.get("hevc").is_some_and(|v| matches!(v.as_str(), "0" | "false"));
    let force_h264 = q.get("video").is_some_and(|v| v == "h264") || (no_hevc && p.video_codec.as_deref() == Some("hevc"));
    let wanted = crate::scan::load_settings(&st)?.filter.audio_lang;
    let track = audio.unwrap_or_else(|| default_audio(&p.audio, &wanted));
    if p.direct && t == 0.0 && track == 0 && !force_h264 {
        return serve(&st, &e, index, &path, size, &headers).await;
    }
    let ffmpeg = tool(&st, "ffmpeg.exe").ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "ffmpeg is missing (tools/ffmpeg)"))?;
    let mut cmd = command(ffmpeg);
    cmd.args(["-v", "error", "-nostdin"]);
    if t > 0.0 {
        cmd.args(["-ss", &format!("{t:.3}")]);
    }
    cmd.arg("-i").arg(file_url(&st, &id, index));
    cmd.args(["-map", "0:v:0?", "-map", &format!("0:a:{track}?"), "-sn", "-dn"]);
    if p.copy_video && !force_h264 {
        cmd.args(["-c:v", "copy"]);
        if p.video_codec.as_deref() == Some("hevc") {
            cmd.args(["-tag:v", "hvc1"]);
        }
    } else {
        cmd.args(h264_args(h264_encoder(&st).await));
    }
    match p.audio.get(track) {
        Some(a) if a.codec == "aac" && a.channels <= 2 => cmd.args(["-c:a", "copy"]),
        _ => cmd.args(["-c:a", "aac", "-ac", "2", "-b:a", "192k"]),
    };
    cmd.args(["-avoid_negative_ts", "make_zero", "-movflags", "frag_keyframe+empty_moov+default_base_moof", "-f", "mp4", "pipe:1"]);
    let mut child = cmd.spawn().map_err(|_| ApiError::internal("cannot run ffmpeg"))?;
    let stdout = child.stdout.take().ok_or_else(|| ApiError::internal("cannot run ffmpeg"))?;
    // The child lives inside the body stream: a closed player connection drops and kills it.
    let stream = futures_util::stream::unfold((ReaderStream::with_capacity(stdout, 256 * 1024), child), |(mut reader, child)| async move {
        futures_util::StreamExt::next(&mut reader).await.map(|chunk| (chunk, (reader, child)))
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "video/mp4")
        .header(header::ACCEPT_RANGES, "none")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .map_err(|_| ApiError::internal("response"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_range("bytes=90-500", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=5-1", 100), None);
        assert_eq!(parse_range("items=0-1", 100), None);
    }

    #[test]
    fn decides_between_direct_play_and_ffmpeg() {
        let stream = |kind: &str, codec: &str| json!({ "codec_type": kind, "codec_name": codec, "channels": 6, "width": 1920, "height": 1080, "disposition": { "default": 1 }, "tags": { "language": "ger" } });
        let probe = |ext: &str, v: &str, a: &str| probe_from(&json!({ "format": { "duration": "888.05" }, "streams": [stream("video", v), stream("audio", a)] }), ext);

        let mp4 = probe("mp4", "h264", "aac");
        assert!(mp4.direct && mp4.copy_video);
        assert_eq!((mp4.duration, mp4.height, mp4.audio[0].lang.as_deref()), (Some(888.05), Some(1080), Some("ger")));
        // MKV always goes through ffmpeg; H.264/HEVC video is copied, only the audio is converted.
        let mkv = probe("mkv", "hevc", "ac3");
        assert!(!mkv.direct && mkv.copy_video);
        assert!(!probe("mp4", "h264", "dts").direct);
        let old = probe("avi", "mpeg4", "mp3");
        assert!(!old.direct && !old.copy_video);
    }

    #[test]
    fn lists_tracks_and_picks_the_wanted_audio_language() {
        let p = probe_from(
            &json!({ "format": { "duration": "60.0" }, "streams": [
                { "codec_type": "video", "codec_name": "hevc", "width": 3840, "height": 2160 },
                { "codec_type": "audio", "codec_name": "eac3", "channels": 6, "disposition": { "default": 1 }, "tags": { "language": "eng", "title": "English 5.1" } },
                { "codec_type": "audio", "codec_name": "ac3", "channels": 6, "disposition": { "default": 0 }, "tags": { "language": "ger" } },
                { "codec_type": "subtitle", "codec_name": "subrip", "disposition": { "default": 1, "forced": 0 }, "tags": { "language": "ger", "title": "Deutsch" } },
                { "codec_type": "subtitle", "codec_name": "hdmv_pgs_subtitle", "disposition": { "default": 0, "forced": 1 }, "tags": { "language": "eng" } },
            ] }),
            "mkv",
        );
        assert_eq!(p.audio.len(), 2);
        assert_eq!((default_audio(&p.audio, "de"), default_audio(&p.audio, "en"), default_audio(&p.audio, "any")), (1, 0, 0));
        assert_eq!(default_audio(&[], "de"), 0);
        let s = &p.subtitles;
        assert_eq!((s.len(), s[0].index, s[0].lang.as_deref(), s[0].title.as_deref(), s[0].text, s[0].default), (2, 0, Some("ger"), Some("Deutsch"), true, true));
        assert!(!s[1].text && s[1].forced && s[1].index == 1);
    }

    #[test]
    fn measures_the_buffer_ahead_of_the_play_position() {
        // 8 pieces of 100 bytes; the file covers bytes 150..750 of the torrent. Pieces 1, 2, 3 and 6 are there.
        let bits = [0b0111_0010u8];
        let ahead = |pos| buffered_ahead(&bits, 100, 150, 600, pos);
        assert_eq!(ahead(0), 250); // from 150 to the end of piece 3 (400)
        assert_eq!(ahead(200), 50);
        assert_eq!(ahead(250), 0); // piece 4 is missing
        assert_eq!(ahead(450), 100); // piece 6 only; piece 7 is missing
        assert_eq!(ahead(600), 0);
        // The last piece is cut at the file's end.
        assert_eq!(buffered_ahead(&[0xff], 100, 150, 600, 500), 100);
        assert_eq!(buffered_ahead(&[], 100, 0, 600, 0), 0);
        assert_eq!(buffered_ahead(&bits, 0, 0, 600, 0), 0);
    }

    #[test]
    fn sizes_the_readahead_window_by_bitrate() {
        let gib = 1024 * MIB;
        // 2 Mbit/s: two minutes are 30 MB, the engine's own 32 MiB are the floor.
        assert_eq!(window_bytes(250_000, 8 * gib), ENGINE_WINDOW);
        // 16 Mbit/s: two minutes = 240 MB.
        assert_eq!(window_bytes(2_000_000, 8 * gib), 240_000_000);
        // A 4K remux (80 Mbit/s) hits the cap; small files cap at their size; unknown bitrate = 64 MiB.
        assert_eq!(window_bytes(10_000_000, 60 * gib), WINDOW_MAX);
        assert_eq!(window_bytes(2_000_000, 100 * MIB), 100 * MIB);
        assert_eq!(window_bytes(0, 8 * gib), 64 * MIB);
    }

    #[test]
    fn snaps_a_seek_to_the_keyframe_before_it() {
        let lines = "22.291667\n32.267000,\n\n40.600000\n";
        assert_eq!(snap_to_keyframe(lines, 40.0), Some(32.267));
        assert_eq!(snap_to_keyframe(lines, 40.6), Some(40.6));
        assert_eq!(snap_to_keyframe(lines, 10.0), None);
        assert_eq!(snap_to_keyframe("", 10.0), None);
    }

    #[test]
    fn shifts_webvtt_cues_to_the_stream_start() {
        let vtt = "WEBVTT\n\n00:08.867 --> 00:11.867\nErste Zeile\n\n00:17.867 --> 00:21.367 align:start\nZweite\nzweizeilig\n\n01:00:48.867 --> 01:00:51.867\nDritte\n";
        let shifted = |offset: f64| {
            let mut shift = VttShift::new(offset);
            vtt.lines().filter_map(|l| shift.line(l)).collect::<Vec<_>>().join("\n")
        };
        // No offset: only the time format is normalised.
        assert_eq!(shifted(0.0), "WEBVTT\n\n00:00:08.867 --> 00:00:11.867\nErste Zeile\n\n00:00:17.867 --> 00:00:21.367 align:start\nZweite\nzweizeilig\n\n01:00:48.867 --> 01:00:51.867\nDritte");
        // 10 s in: the first cue is still on screen (starts at 0), the others move by exactly 10 s.
        assert_eq!(shifted(10.0), "WEBVTT\n\n00:00:00.000 --> 00:00:01.867\nErste Zeile\n\n00:00:07.867 --> 00:00:11.367 align:start\nZweite\nzweizeilig\n\n01:00:38.867 --> 01:00:41.867\nDritte");
        // Past the first two cues: they are dropped with their text.
        assert_eq!(shifted(32.267), "WEBVTT\n\n01:00:16.600 --> 01:00:19.600\nDritte");
        assert_eq!((vtt_time("01:02:03.500"), vtt_time("02:03.5"), vtt_time("x")), (Some(3723.5), Some(123.5), None));
    }

    #[test]
    fn h264_encoder_arguments() {
        let amf = h264_args("h264_amf");
        assert!(amf.starts_with(&["-c:v", "h264_amf"]) && amf.ends_with(&["-pix_fmt", "nv12"]));
        assert!(h264_args("libx264").contains(&"veryfast"));
    }
}
