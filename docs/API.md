# Flimmer API (Phase 1–3)

Server: Rust (axum) on `http://127.0.0.1:43300`. All JSON. All TMDB texts in `config.language` (default `de-DE`).
Image fields are full URLs (`https://image.tmdb.org/t/p/w500/...` for posters, `.../w1280/...` for backdrops, `.../w300/...` for stills/profiles) or `null`.
Errors: HTTP 4xx/5xx with `{ "error": "message" }`.

## Types

```ts
type MediaType = "movie" | "tv";

interface Item {            // card in any list
  id: number;               // TMDB id
  type: MediaType;
  title: string;            // movie.title or tv.name
  year: number | null;
  poster: string | null;
  backdrop: string | null;
  rating: number | null;    // vote_average, 1 decimal, null if 0 votes
  overview: string;
}

interface Details extends Item {
  original_title: string;
  imdb_id: string | null;
  genres: { id: number; name: string }[];
  runtime: number | null;   // minutes (movie runtime, tv: first episode_run_time or null)
  tagline: string | null;
  trailer_youtube_key: string | null; // prefer de trailer, fallback en
  seasons: Season[];        // tv only, excluding season 0 ("Specials") unless it is the only one; [] for movies
  cast: { name: string; character: string; profile: string | null }[]; // max 12
  recommendations: Item[];  // max 20
}

interface Season { season_number: number; name: string; episode_count: number; poster: string | null; air_date: string | null }

interface SeasonDetails {
  season_number: number;
  name: string;
  episodes: { episode_number: number; name: string; overview: string; still: string | null; air_date: string | null; runtime: number | null }[];
}

interface Source { id: string; name: string; no_ads: boolean; url: string }
```

## Endpoints

| Method | Path | Returns |
|---|---|---|
| GET | `/api/status` | `{ tmdb_ok: boolean, version: string }` |
| GET | `/api/home` | `{ trending: Item[], popular_movies: Item[], popular_tv: Item[], top_rated_movies: Item[], top_rated_tv: Item[] }` (trending = TMDB trending/all/day, people filtered out) |
| GET | `/api/list/:type/:list?page=1&genre=<id>` | `{ results: Item[], page, total_pages }`; `:list` = `popular` \| `top_rated` \| `trending` \| `now_playing` (movie) \| `on_the_air` (tv). With `genre` → TMDB discover sorted by popularity. |
| GET | `/api/genres/:type` | `{ id, name }[]` |
| GET | `/api/search?q=<text>&page=1` | `{ results: Item[], page, total_pages }` (TMDB search/multi, people filtered out) |
| GET | `/api/title/:type/:id` | `Details` |
| GET | `/api/title/tv/:id/season/:n` | `SeasonDetails` |
| GET | `/api/sources/:type/:id?s=1&e=1` | `Source[]` in `providers.json` order. Templates: `{tmdb}` `{imdb}` `{s}` `{e}`; providers whose template needs `{imdb}` are skipped if the title has no imdb id. `s`/`e` required for tv. |
| GET | `/api/watchlist` | `Item[]` newest first |
| PUT | `/api/watchlist/:type/:id` | body `Item` → `204` |
| DELETE | `/api/watchlist/:type/:id` | `204` |

## Phase 2: scan, own player, prefetch, settings

The stream sniffer only exists when the server runs inside the Flimmer desktop app (Tauri). Standalone `flimmer.exe` (browser access) has no sniffer: scan endpoints return state `"unavailable"`, but already cached scan results and streams still work.

```ts
interface Caps { sniffer: boolean }

interface Variant { width: number | null; height: number | null; bandwidth: number | null; codecs: string | null }
interface Track { lang: string | null; name: string | null }   // lang = ISO 639 code as given by the playlist ("eng", "ger", "und" …)

interface ScanResult {
  provider_id: string; name: string;
  status: "pending" | "running" | "ok" | "no_stream" | "error";
  ms: number | null;               // time until the stream was captured
  max_width: number | null; max_height: number | null; max_bandwidth: number | null;
  variants: Variant[];             // sorted best first; single-variant/mp4 streams → one entry (dims may be null)
  audio: Track[]; subtitles: Track[];
  stream_id: string | null;        // set when status = "ok"
  kind: "hls" | "mp4" | null;      // set when status = "ok"; "mp4" = plain file: play master.m3u8 as <video src> (no hls.js), no prefetch
  error: string | null;            // reason for "error" (e.g. "provider host not found"), optional hint for "no_stream"
}

interface Scan { state: "idle" | "running" | "done" | "unavailable"; started_at: string | null; finished_at: string | null; results: ScanResult[] }

interface Settings {
  filter: {
    min_height: 0 | 480 | 720 | 1080 | 2160;   // 0 = egal
    audio_lang: "any" | "de" | "en";           // preferred audio language (matches ger/deu/de, eng/en)
    sort: "quality" | "bitrate" | "speed";     // tie-break for auto pick
  };
  prefetch_minutes: number;                    // default 10
}

interface BufferState { variant: number; from_seconds: number; ahead_seconds: number; bytes: number; done: boolean }
```

| Method | Path | Returns |
|---|---|---|
| GET | `/api/caps` | `Caps` |
| POST | `/api/scan/:type/:id?s=&e=` | `202` starts a scan of all providers (reuses results younger than 6 h unless `&force=1`; `&force=1&provider=<provider_id>` re-scans only that provider and keeps the rest); `Scan` body. Without sniffer: `200` with state `"unavailable"` |
| GET | `/api/scan/:type/:id?s=&e=` | `Scan` (UI polls ~1 s while `running`; results fill in as providers finish) |
| GET | `/api/stream/:stream_id/master.m3u8` | proxied master playlist, all URIs rewritten to go through the server (correct Referer/Origin/UA added server-side). For `kind: "mp4"` streams: `307` to the proxied file |
| GET | `/api/stream/:stream_id/r?u=<encoded>` | proxied sub-playlist / segment / key / subtitle (served from prefetch cache when present; response header `x-flimmer-cache: hit` \| `miss`) |
| POST | `/api/stream/:stream_id/prefetch?variant=<i>&from=<seconds>` | `204`; downloads segments of that variant from `from` up to `from + prefetch_minutes` to `data/cache/<stream_id>/`, cancels the previous prefetch of that stream. `variant` = index into `ScanResult.variants`. `400` for mp4 streams or a bad index, `410` when expired |
| GET | `/api/stream/:stream_id/buffer` | `BufferState` |
| DELETE | `/api/stream/:stream_id` | `204`; stops prefetch, deletes its cache |
| GET | `/api/settings` | `Settings` |
| PUT | `/api/settings` | body `Settings` → `204` (stored in SQLite) |

A stream whose upstream starts failing (403/410, expired token) → `/api/stream/...` returns `410 { "error": "expired" }`; the UI then re-scans that provider with `force=1`.
Cache housekeeping: on start, delete `data/cache/*` older than 24 h.

Scan order and skipping (no new fields; `results` always stays in `providers.json` order, numbers in `docs/scan-speed.md`):
- Up to 10 provider pages load at once, started 0.3 s apart. Per media type the server remembers each provider's outcomes (SQLite table `provider_health`). Queue order: providers with a stream in their last scan (fastest first), then never scanned ones, then ones that failed last time.
- A provider that had a stream in its last scan and shows none now is tried a second time before its result is reported (it stays `running` meanwhile).
- A provider without a stream in 3 full scans in a row is skipped: its result appears immediately with its last status (`no_stream` or `error`) and `error: "skipped: … in the last N scans (tried again once a day)"`. 24 h after its last try it is scanned again, at the end of the queue. `&force=1&provider=<provider_id>` always scans that provider. Failures only count in a scan in which at least one other provider had a stream.
- A scan window gives up after 15 s when the page made no media request at all, otherwise 6 s after the last one, at most after 25 s.

## Phase 3: watch progress, Weiterschauen, Verlauf, Neue Folgen

A movie is addressed as `/:type/:id`, an episode as `/tv/:id?s=&e=` (`s`/`e` required for tv, like `/api/sources`). Positions are seconds. Timestamps are ISO 8601 UTC (`2026-11-20T03:46:31Z`).
**Finished** = position ≥ 90 % of the duration, or less than 3 min left (only for titles of 10 min or more).
Stored in SQLite: `progress` (one row per movie/episode) and `history` (watching sessions; watching the same movie/episode again within 6 h updates its entry instead of adding one).

```ts
interface WatchMeta { item: Item; episode_name?: string | null; still?: string | null } // stored with the row so lists need no TMDB call; item = the movie / the series

interface Progress {
  type: MediaType; id: number;
  season: number | null; episode: number | null;   // null for movies
  position: number; duration: number; finished: boolean; updated_at: string;
}

interface ContinueEntry {
  item: Item;
  season: number | null; episode: number | null;
  episode_name: string | null; still: string | null;
  position: number; duration: number;              // 0 / 0 for a not yet started next episode
  up_next: boolean;                                // previous episode finished (or this one barely started): play from the start
  updated_at: string;
}

interface HistoryEntry {
  entry: number;                                   // id for DELETE
  item: Item; season: number | null; episode: number | null;
  episode_name: string | null; still: string | null;
  position: number; duration: number; finished: boolean; watched_at: string;
}

interface NewEpisode {
  item: Item;                                      // the series
  season: number; episode: number;                 // first new episode
  name: string; still: string | null; air_date: string; // YYYY-MM-DD
  count: number;                                   // new episodes of this series, this one included
}
```

| Method | Path | Returns |
|---|---|---|
| GET | `/api/progress/:type/:id` | `Progress[]` of that title (tv: every episode with a row, in episode order) |
| PUT | `/api/progress/:type/:id?s=&e=` | body `WatchMeta & { position, duration }` → `204`. Server decides `finished`; a save brings the title back into Weiterschauen. Adds/updates a history entry when position ≥ 30 s or finished. UI saves every 10 s while playing, on pause, seek, end and when leaving (`fetch` with `keepalive`) |
| PUT | `/api/watched/:type/:id?s=&e=` | body `WatchMeta` → `204`; "Als gesehen markieren" (finished, no history entry) |
| DELETE | `/api/watched/:type/:id?s=&e=` | `204`; "Ungesehen": deletes the progress row |
| GET | `/api/continue` | `ContinueEntry[]`, max 20, most recent first. One entry per title: unfinished movie (≥ 30 s), unfinished episode, or for a finished episode the next aired one of that series (skips episodes already marked watched, crosses into the next season; TMDB tv details + season, cached 6 h) |
| DELETE | `/api/continue/:type/:id` | `204`; hides the title from Weiterschauen (positions are kept; the next save shows it again) |
| GET | `/api/history` | `HistoryEntry[]`, newest first, max 500 |
| DELETE | `/api/history/:entry` | `204`; removes one entry |
| DELETE | `/api/history` | `204`; clears the history (progress and "gesehen" marks stay) |
| GET | `/api/new-episodes` | `NewEpisode[]`, newest air date first. Series = tv titles with progress (furthest episode seen) plus tv titles on the watchlist (max 60). New = episodes of the season of TMDB `last_episode_to_air` that aired in the last 30 days and come after the furthest episode seen. TMDB data cached 6 h |

Everything not under `/api/` serves the built web UI (`web/dist`), unknown paths fall back to `index.html` (SPA routing).

## Languages (lang parameter)

Every `/api` call accepts `?lang=xx-YY`; details in [i18n.md](i18n.md).

## Phase 5: Torrents (search, streaming, download) behind the VPN

Search sources follow [melqtx/tork](https://github.com/melqtx/tork) (MIT), plus a Torrentio lookup by IMDb id. Engine: embedded `librqbit`.

**Network rule.** Torrent traffic exists only inside a WireGuard tunnel that Flimmer opens itself (userspace: `boringtun` + `smoltcp`, no adapter, no admin rights, nothing else on the PC is routed through it). The engine's only ways out are a loopback SOCKS5 server that dials through the tunnel (peers) and an in-process tracker shim; the real tracker announces (UDP + HTTP) and their DNS lookups are made by Flimmer through the tunnel. DHT, local peer discovery, UPnP and incoming connections are off. Tunnel down = SOCKS refuses = no torrent traffic (kill-switch, there is no fallback route). The tunnel opens when the first torrent becomes active (not paused, not completed) and closes 5 s after the last one stops. A finished download is paused in the engine (no seeding); upload while downloading is limited (`upload_kbps`). Searching uses the normal network (plain HTTPS to the index sites).
The system must let Flimmer reach the VPN server: while the Proton VPN app is connected with its kill switch, the tunnel cannot handshake (`Vpn.error`).

```ts
interface Release {                    // parsed from the release name
  title: string; year: number | null;
  season: number | null; episode: number | null; pack: boolean;   // pack = whole season
  height: 480 | 576 | 720 | 1080 | 2160 | null;
  source: "Remux" | "BluRay" | "WEB-DL" | "WEBRip" | "HDTV" | "DVD" | "CAM" | null;
  codec: "HEVC" | "H.264" | "AV1" | "XviD" | null;
  hdr: "DV" | "HDR10+" | "HDR" | null;
  audio: "Atmos" | "TrueHD" | "DTS-HD" | "DTS" | "EAC3" | "AC3" | "AAC" | "FLAC" | "Opus" | null;
  languages: string[];                 // "de", "en", "multi", "fr" …; [] = not stated (usually English). "German DL" → ["de","en"]
}

interface TorrentResult {
  infohash: string;                    // 40 hex, lowercase
  name: string; size: number;          // bytes
  seeders: number;                     // summed over all sources listing this infohash
  seeders_max: number;                 // highest value of a single source
  leechers: number;
  sources: string[];                   // source ids
  trackers: string[];
  file_index: number | null;           // file of the requested movie/episode inside the torrent (season packs); pass it to /start
  exact: boolean;                      // found by IMDb id
  release: Release;
  below_min: boolean;                  // misses Settings.filter.min_height
  score: number;                       // sort key; results are sorted best first
  magnet: string;
}

interface SourceStatus { id: string; status: "ok" | "blocked" | "error" | "skipped"; count: number; ms: number; error: string | null }
interface TorrentSearch { results: TorrentResult[]; sources: SourceStatus[] }   // max 120 results, cached 5 min

interface Vpn {
  state: "down" | "connecting" | "up" | "blocked";   // blocked = kill-switch engaged by POST /api/vpn/kill
  since: string | null; public_ip: string | null; country: string | null;   // address seen through the tunnel (ipinfo.io asked through it)
  handshake_age: number | null;        // seconds since the last WireGuard handshake
  rx_bytes: number; tx_bytes: number; error: string | null; config_found: boolean;
}

interface TorrentMedia { type: MediaType; id?: number; title: string; year?: number; season?: number; episode?: number }

interface Torrent {
  id: string;                          // infohash
  name: string; mode: "stream" | "download"; media: TorrentMedia | null;
  state: "metadata" | "starting" | "checking" | "downloading" | "paused" | "completed" | "error" | "vpn_blocked";
  progress: number;                    // 0..1 of the selected files
  downloaded: number; total: number; down_speed: number; up_speed: number;   // bytes, bytes/s
  peers: number; peers_known: number; eta_seconds: number | null;
  folder: string; error: string | null; added_at: string;
  files: { index: number; name: string; size: number; path: string; selected: boolean; downloaded: number; video: boolean }[];  // [] until the metadata is there
}

interface PlayInfo {
  file: number; name: string; size: number;
  mode: "direct" | "transcode";
  url: string;                         // /api/torrents/:id/play?file=N
  duration: number | null;             // seconds
  video_codec: string | null; width: number | null; height: number | null; copy_video: boolean;
  audio: { index: number; codec: string; channels: number; lang: string | null; title: string | null; default: boolean }[];
}

interface TorrentSettings { vpn_config: string; movies_dir: string; series_dir: string; upload_kbps: number }
```

| Method | Path | Returns |
|---|---|---|
| GET | `/api/torrents/search?q=<text>` | `TorrentSearch`, free search (Torrentio/EZTV are `skipped`: they need an IMDb id) |
| GET | `/api/torrents/:type/:id?s=&e=` | `TorrentSearch` for a movie / episode: IMDb lookups (Torrentio, YTS, EZTV) plus text search with the original and the localized title; text hits must match title, year (±1) or season/episode (season packs included) |
| POST | `/api/torrents/start` | body `{ magnet?: string, infohash?: string, trackers?: string[], name?: string, mode?: "stream" \| "download", file_index?: number, files?: number[], media?: TorrentMedia }` → `202` `Torrent`. Opens the tunnel, resolves the metadata, picks the files (`files`/`file_index`, else the episode in `media`, else all videos for a tv `media` without episode, else the largest video) and starts. Starting a known torrent again resumes it and applies `files`. Stream and download both save to disk; "stream" only means the UI plays it right away |
| GET | `/api/torrents` | `{ vpn: Vpn, torrents: Torrent[] }`, newest first (poll ~1–2 s while something is active) |
| GET | `/api/torrents/:id` | `Torrent` |
| POST | `/api/torrents/:id/pause` | `204` |
| POST | `/api/torrents/:id/resume` | `204` |
| PUT | `/api/torrents/:id/files` | body `{ files: number[] }` → `204`; changes the selection (season packs) |
| POST | `/api/torrents/:id/open` | `204`; opens the torrent's folder in Explorer on the PC Flimmer runs on, with the first chosen video selected when it exists. `404` while the folder does not exist yet |
| DELETE | `/api/torrents/:id?files=1` | `204`; with `files=1` also deletes the torrent's files (and its folders when empty) |
| GET | `/api/torrents/:id/playinfo?file=<i>` | `PlayInfo` (ffprobe; waits until the file's start is downloaded, `504` after 90 s). `file` defaults to the first selected video |
| GET | `/api/torrents/:id/play?file=<i>&t=<seconds>&audio=<i>&video=h264` | the playable stream, see below |
| GET | `/api/torrents/:id/file/:index` | raw file with HTTP range support |
| GET | `/api/torrents/settings` | `TorrentSettings` (defaults: `<root>/data/vpn/proton.conf`, `D:\Flimmer Downloads\Filme`, `D:\Flimmer Downloads\Serien`, 30) |
| PUT | `/api/torrents/settings` | body `TorrentSettings` → `204` |
| GET | `/api/vpn` | `Vpn` |
| POST | `/api/vpn/kill` | `204`; kill-switch: tunnel down and held down, all torrents show `vpn_blocked` and have no traffic |
| POST | `/api/vpn/release` | `204`; the tunnel comes back when a torrent is active |

**Folders.** Movie: `<movies_dir>\<Titel (Jahr)>\`, series: `<series_dir>\<Titel>\Staffel NN\`, without `media`: `<movies_dir>\<torrent name>\`; inside, the torrent's own file paths.

**Playing.** `PlayInfo.mode` tells the player what `/play` returns:
- `direct` (MP4/WebM with H.264/HEVC/VP9/AV1 and AAC/MP3/Opus): the file itself with range support. Use it as `<video src>`; seeking is native and the engine prioritises the requested position. Works while downloading and offline afterwards.
- `transcode` (MKV, AC3/DTS/EAC3/TrueHD audio, old video codecs): ffmpeg (`tools/ffmpeg/bin`, gyan.dev build, not in git) writes fragmented MP4 on the fly: video copied when `copy_video` (else H.264), audio → AAC stereo. Also a plain `<video src>`, but the stream is not seekable: to seek, load `/play?...&t=<seconds>` and add `t` to `currentTime`; use `PlayInfo.duration` for the timeline. `audio=<i>` picks the audio track (German/English in "DL" releases), `video=h264` forces a video re-encode if the copied codec does not play.

**Ranking.** `score` = language (wanted `audio_lang` found ≫ `multi`) + quality by `sort` (`quality`: height and source, `bitrate`: size, `speed`: seeders) + seeders (log scale) − penalties (no seeders, below `min_height`, CAM).

## Setup, preferences and data (1.0)

Flimmer runs without `config.json`. Until a TMDB token is stored, every TMDB-backed endpoint answers
`503 { "error": "...", "code": "needs_setup" }` and `/api/status` has `"needs_setup": true`; the UI then shows the
first-run setup. A legacy `config.json` (in the data folder or the resource folder) still works: its token is copied
into the database once and that install counts as set up (no onboarding, no tour). The token is stored in SQLite
(`settings` key `tmdb_token`) and never sent back to the UI.

| Method | Path | Answer |
|---|---|---|
| GET | `/api/setup` | `{ tmdb: "set" \| "missing", onboarded, vpn_file, version, defaults: { movies_dir, series_dir } }` |
| PUT | `/api/setup/tmdb` | JSON `{ token }`; checked with TMDB (`/authentication`), stored only if accepted → `204`; errors with `code`: `format`/`api_key` (400), `rejected` (422), `unreachable` (502) |
| POST | `/api/setup/done` | `204`, onboarding finished |
| PUT | `/api/setup/vpn` | JSON `{ text }` (WireGuard file, max 8 KB) → checked, written to `<data>/vpn/proton.conf`, torrent settings point at it → `204`. Content is never echoed or logged. |
| DELETE | `/api/setup/vpn` | removes that file, tunnel down → `204` |
| POST | `/api/setup/open` | JSON `{ page }` with a fixed id (`tmdb_signup`, `tmdb_api`, `proton_wireguard`, license pages) → opens it in the system browser |
| POST | `/api/setup/reset` | JSON `{ confirm: "reset" }` → empties watchlist, progress, history, settings (token, prefs), scans, health; deletes the VPN file. Torrents and downloaded files stay. |
| GET / PUT | `/api/prefs` | UI preferences object (max 16 KB), opaque to the server |
| POST | `/api/export` | writes `Flimmer-Einstellungen-<date>.json` (prefs, source settings, torrent folders/upload, watchlist; never token or VPN file) into the user's Downloads folder → `{ path }` |
| POST | `/api/import` | body = such a file → applies each part with the normal validation, merges the watchlist → `{ watchlist: n }` |
| GET | `/api/maintenance` | `{ stream_cache_bytes, scan_profile_bytes, scan_profile_pending, data_dir }` |
| DELETE | `/api/maintenance/cache` | clears the stream cache and saved scan results now, the source-search browser profile on the next start |

`Settings` (`/api/settings`) has `parallel` (4..=12, default 10): provider pages loading at the same time in a scan.

All POST/PUT bodies above must be `application/json`, so another web page cannot send them (CORS preflight).

**Folders.** Development (`--root`): data in `<root>/data` as before. Installed: resources next to `Flimmer.exe`,
data in `%LOCALAPPDATA%\local.flimmer.desktop\data` (the folder the uninstaller offers to delete). `--data <dir>`
overrides it. The desktop app uses the port of its last start, else 43300, else the next free one up to 43399
(`--port` pins one), and refuses a second start on the same data folder.
