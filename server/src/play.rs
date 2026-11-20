//! Playback of torrent files. Browser-playable files (MP4/WebM with H.264/HEVC/VP9/AV1 + AAC/MP3/Opus)
//! are served directly with HTTP range support (native seeking; the torrent engine prioritises the
//! requested position). Everything else (MKV, AC3/DTS/EAC3 audio …) goes through ffmpeg on the fly as
//! fragmented MP4: video is copied when the browser can decode it, audio becomes AAC stereo.

use std::{
    io::SeekFrom,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt},
    process::Command,
};
use tokio_util::io::ReaderStream;

use crate::{
    api::{AppState, Params, St},
    error::{ApiError, ApiResult},
    torrent::{Entry, is_video},
};

const COPY_VIDEO: &[&str] = &["h264", "hevc", "vp8", "vp9", "av1"];
const DIRECT_AUDIO: &[&str] = &["aac", "mp3", "opus", "vorbis", "flac"];
const DIRECT_EXT: &[&str] = &["mp4", "m4v", "mov", "webm"];

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
pub struct Probe {
    pub duration: Option<f64>,
    pub video_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub audio: Vec<AudioTrack>,
    /// Plays as it is in the browser.
    pub direct: bool,
    /// ffmpeg would copy the video stream (no re-encode).
    pub copy_video: bool,
}

fn tool(_st: &AppState, name: &str) -> Option<PathBuf> {
    let dir = crate::paths::tools().join("ffmpeg");
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

async fn serve(e: &Entry, index: usize, path: &str, size: u64, headers: &HeaderMap) -> ApiResult<Response> {
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
    reader.seek(SeekFrom::Start(start)).await.map_err(|_| ApiError::internal("seek failed"))?;
    let len = if size == 0 { 0 } else { end - start + 1 };
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
    serve(&e, index, &path, size, &headers).await
}

/// `GET /api/torrents/:id/playinfo?file=`
pub async fn playinfo(State(st): St, Path(id): Path<String>, Query(q): Params) -> ApiResult<Json<Value>> {
    let e = st.torrents.entry(&id)?;
    let (index, path, size) = pick_file(&e, &q)?;
    let p = probe(&st, &e, index, &path).await?;
    Ok(Json(serde_json::json!({
        "file": index, "name": path, "size": size,
        "mode": if p.direct { "direct" } else { "transcode" },
        "url": format!("/api/torrents/{id}/play?file={index}"),
        "duration": p.duration, "video_codec": p.video_codec, "width": p.width, "height": p.height,
        "copy_video": p.copy_video, "audio": p.audio,
    })))
}

/// `GET /api/torrents/:id/play?file=&t=&audio=&video=h264`
pub async fn play(State(st): St, Path(id): Path<String>, Query(q): Params, headers: HeaderMap) -> ApiResult<Response> {
    let e = st.torrents.entry(&id)?;
    let (index, path, size) = pick_file(&e, &q)?;
    let number = |k: &str| q.get(k).map(|v| v.parse::<f64>().map_err(|_| ApiError::bad_request(format!("'{k}' must be a number")))).transpose();
    let t = number("t")?.unwrap_or(0.0).max(0.0);
    let audio = number("audio")?.map(|a| a as usize);
    let force_h264 = q.get("video").is_some_and(|v| v == "h264");

    let p = probe(&st, &e, index, &path).await?;
    if p.direct && t == 0.0 && audio.is_none_or(|a| a == 0) && !force_h264 {
        return serve(&e, index, &path, size, &headers).await;
    }
    let ffmpeg = tool(&st, "ffmpeg.exe").ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "ffmpeg is missing (tools/ffmpeg)"))?;
    let track = audio.unwrap_or_else(|| p.audio.iter().find(|a| a.default).map_or(0, |a| a.index));
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
        cmd.args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "22", "-pix_fmt", "yuv420p"]);
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
}
