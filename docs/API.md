# Flimmer API

Server: Rust (axum) on `http://127.0.0.1:43300`. All JSON. TMDB texts in the `lang` of the call (see "Languages" below), else `de-DE`.
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
| GET | `/api/stream/:stream_id/r?u=<encoded>&s=<signature>` | proxied sub-playlist / segment / key / subtitle (served from prefetch cache when present; response header `x-flimmer-cache: hit` \| `miss`) |
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

## Phase 6: filters and facets, faster swarms, tracks, queue, auto-download

Numbers: [torrent-p6.md](torrent-p6.md). Everything in Phase 5 still holds unless stated here.

**Network rule (replaces the DHT / incoming part of Phase 5).** Still nothing outside the tunnel. New inside it: a client-only DHT (BEP 5, read-only) over the tunnel's UDP, whose peers reach the engine through the tracker shim like tracker peers; the public tracker list (ngosang `trackers_all`, fetched through the tunnel, cached 24 h in `data/cache/trackers.txt`, up to 60 trackers per torrent); a forwarded port asked from the VPN server by NAT-PMP through the tunnel (renewed every 45 s) with a listener inside the tunnel that hands incoming peers to the engine's loopback-only listener. PEX runs inside the peer connections. The engine's own DHT, UPnP and local discovery stay off; a stored torrent file that names any tracker but the shim (or a web seed) is not given to the engine. 150 peer slots per torrent.

```ts
interface Release {                    // Phase 5 fields plus:
  height: 480 | 576 | 720 | 1080 | 2160 | 4320 | null;
  source: "Remux" | "BluRay" | "WEB-DL" | "WEBRip" | "HDTV" | "DVD" | "SCR" | "CAM" | "TS" | "TC" | null;
  cam: boolean;                        // CAM, TS, TC or screener
  codec: "HEVC" | "H.264" | "AV1" | "XviD" | "VC-1" | "MPEG-2" | null;
  bit_depth: 8 | 10 | 12 | null;
  hdr: string | null;                  // best of hdr_all
  hdr_all: ("DV" | "HDR10+" | "HDR10" | "HDR" | "HLG")[];
  audio: "Atmos" | "TrueHD" | "DTS-HD MA" | "DTS-HD" | "DTS" | "EAC3" | "AC3" | "AAC" | "FLAC" | "Opus" | "MP3" | "PCM" | null;
  channels: "7.1" | "5.1" | "2.0" | "1.0" | null;
  languages: string[];                 // audio: "de", "en", "fr", "it", "es", "ja", "ko" …, "multi", "nordic"
  subtitles: string[];                 // "German Subbed", "VOSTFR", "ESubs" …
  dual: boolean;                       // "German DL" (German + original), "Dual Audio"
  dub: "mic" | "line" | "dubbed" | null;   // MD / LD / AC3D, Dubbed
  group: string | null; proper: boolean; repack: boolean;
  edition: "Extended" | "Uncut" | "Director's Cut" | "Unrated" | "IMAX" | "Remastered" | "Theatrical" | null;
  "3d": boolean;
}

// TorrentResult plus: added: number | null (upload time, unix seconds, earliest source); age_days: number | null
// TorrentSearch plus:
//   facets: { [dimension]: { [value]: count } }   // dimensions: language, sub, res, source, codec, hdr, audio, channels, group (top 30), cam
//   total: number      // merged results before filters
//   matched: number    // after filters (results holds the best 120 of them)
```

**Filters** on `/api/torrents/search` and `/api/torrents/:type/:id` (comma list = any of them; different parameters must all match):
`language` (`de`, `en`, …, `und` = no language in the name), `sub`, `res` (`2160`, `1080`, `720`, `sd`, `unknown`), `source`, `codec`, `hdr` (`SDR` = none), `audio`, `channels`, `group`, `min_seeders`, `max_size`, `min_size` (bytes), `exclude_cam` (default `true`; `false` shows CAM/TS/TC/screeners). The language filter is `language`, not `lang`: `lang` is the TMDB language of every request. Each facet counts with all *other* filters applied, so a number says how many results choosing that value shows. `facets.cam.true` = hidden CAM copies.
`mode=fast` ranks for a quick start (many seeders, small file), `mode=quality` for the best release (a healthy swarm is enough, source and resolution decide); without `mode` the saved `sort` applies.

```ts
// Torrent plus:
//   state also "queued"; priority: number (-100..100); queue_position: number | null; auto: boolean (queued by a rule)
//   peers_by_source / known_by_source: { tracker, dht, pex, incoming }   // connected / known to the engine
//   found_by_source: { tracker, dht }                                     // addresses found
//   files[]: plus season: number | null, episode: number | null          // season packs
//   stream_health: { file, playing, position_bytes, position_seconds | null, buffered_bytes, buffered_seconds | null,
//                    bitrate | null (bit/s), window_bytes, peers, down_speed } | null
// Vpn plus: forwarded_port: number | null; port_forward_error: string | null; incoming_total; incoming_live; dht_nodes
// TorrentSettings plus: max_active: number (1..10, default 3); min_free_gb: number (default 5)
// PlayInfo plus: default_audio: number (saved audio_lang, else the file's default); hevc: boolean;
//   subtitles: { index, codec, lang, title, default, forced, text }[];   // text = can be served as WebVTT
//   h264_encoder: "h264_amf" | "libx264"; next_file: { file, season, episode } | null

interface TorrentEvent { id: number; at: string; kind: "finished" | "failed" | "grabbed"; torrent_id: string; name: string; media: TorrentMedia | null; message: string | null }

interface AutoRule {
  type: MediaType; id: number; title: string; year: number | null; enabled: boolean;
  min_height: 0 | 480 | 720 | 1080 | 2160; max_height: 0 | 480 | 720 | 1080 | 2160;   // 0 = any
  lang: string;                        // "any" | "de" | "en" …
  sources: string[];                   // allowed Release.source values, [] = any
  max_size_gb: number; min_seeders: number;   // 0 = no size limit; default 3 seeders
  since: string;                       // series: episodes aired on or after this day (default: the day the rule was made)
  created_at: number; on_watchlist: boolean;
}
interface Grab { key: string; type: MediaType; id: number; title: string; season: number | null; episode: number | null; infohash: string; name: string; size: number; seeders: number; at: string }
interface Decision { rule: string; title: string; season: number | null; episode: number | null; air_date: string | null; action: "grab" | "none" | "skip"; reason: string | null; found: number; candidate: { infohash, name, size, seeders, height, source, languages } | null }
```

| Method | Path | Returns |
|---|---|---|
| POST | `/api/torrents/start` | also takes `priority`. A new `download` waits for a free slot (`queued`); a `stream` starts at once and is never queued. Before it starts, the free space of the target drive is checked: too little → state `error` with the reason and one `failed` event |
| PUT | `/api/torrents/:id/priority` | body `{ priority }` → `204`. Order of the queue: streams, then priority, then age |
| GET | `/api/torrents/events?since=<id>` | `{ events: TorrentEvent[], last: number }`, oldest first, the newest 200 are kept (also over restarts). Poll with the last `last` |
| GET | `/api/torrents/announce?info_hash=…` | internal: the tracker shim the torrent engine announces to (bencoded peers from the tunnel's trackers, DHT and incoming). Loopback requests without `Origin`, `Sec-Fetch-Site` or `Cookie` (the engine) need no session; everything else goes through the normal access check |
| GET | `/api/torrents/:id/next?file=<i>` | the next episode's file in a season pack: `{ torrent_id, file, name, size, season, episode, selected, completed, url, playinfo }`; selects it for download when it is not chosen yet. `404` when there is none |
| GET | `/api/torrents/local/:type/:id?s=&e=` | the same shape plus `state`: a known torrent that already holds this movie / episode. `404` when none does |
| GET | `/api/torrents/:id/subtitle?file=<i>&track=<i>&t=<seconds>` | the subtitle track as WebVTT, cue times moved `t` seconds earlier (the stream's start). `415` for image subtitles, `404` for an unknown track. While the file is still downloading the cues arrive as the data does |
| GET | `/api/torrents/:id/seek?file=<i>&t=<seconds>&video=h264&hevc=0` | `{ t, start, requested, exact }`: pass `t` to `/play`; `start` is the file time of the stream's first frame (timeline offset and `/subtitle?t=`). A copied video stream starts at the keyframe before the asked time; a re-encoded one exactly there |
| GET | `/api/torrents/:id/play?…&hevc=0` | `hevc=0` = the player cannot decode HEVC: that codec is re-encoded to 8-bit H.264 (`PlayInfo.h264_encoder`; `video=h264` uses the same encoder). Without `audio=` the track is `PlayInfo.default_audio` |
| GET | `/api/autodl/rules` | `{ rules: AutoRule[], last_run: string \| null, interval_minutes: 30 }` |
| PUT | `/api/autodl/rules/:type/:id` | body: `AutoRule` fields without `type`, `id`, `created_at`, `on_watchlist` (only `title` is required) → `204` |
| DELETE | `/api/autodl/rules/:type/:id` | `204` |
| POST | `/api/autodl/preview/:type/:id` | same body as the `PUT`; `{ dry: true, decisions: Decision[] }`: what that rule would take now. Stores and queues nothing; the title need not be on the watchlist |
| POST | `/api/autodl/run?dry=1` | runs the job now over all enabled rules; `{ dry, decisions }`. With `dry=1` nothing is queued |
| GET | `/api/autodl/history` | `Grab[]`, newest first |
| DELETE | `/api/autodl/history/:key` | `204`; the job may take that movie / episode again |

**Auto-download.** The job runs 2 minutes after the start and then every 30 minutes, for enabled rules whose title is on the watchlist. A movie is taken once. For a series the due episodes are those of the season of TMDB's last aired episode with an air date from `since` up to today that are not marked watched, not grabbed before and not in a known torrent; at most 5 per series and run. The best release that fits the rule is queued as a `download` (`auto: true`, a `grabbed` event). CAM/TS/screeners, 3D, mic/line-dubbed audio and (for episodes) season packs are never taken.

**Streaming order.** The engine fetches 32 MiB ahead of every open reader. When a stream starts, the file's first 6 MiB and last 4 MiB come first (container header and index). While a file is read, a helper reader sits at the end of what is buffered, so the window grows to about 2 minutes of playback (32 MiB to 512 MiB). A seek is a new request and moves the priority at once.

**Auto-next from a season pack.** `PlayInfo.next_file` and `/next` name the next episode. When a progress save (`PUT /api/progress/tv/:id`) passes the middle of an episode that plays from a pack, the next episode's file is selected and its start fetched.

## Phase 7: more search sources, source switches, torrents without VPN

**Search sources.** 27 sources, listed in this order in `TorrentSearch.sources` (`SourceStatus.status` also `"off"` = turned off in the settings). New: Comet (two instances), TorrentsDB, MediaFusion, Peerflix (Stremio add-ons, IMDb id), BitSearch, Torrents-CSV, TorrentDownloads, TorrentDownload.info, Zilean, BT4G, TorrentKitty, Rutor, and for anime SubsPlease, AnimeTosho, TokyoTosho, DMHY, Mikan. Endpoints and test results: [torrent_quellen_kandidaten.md](torrent_quellen_kandidaten.md); KickassTorrents is left out (magnet only on a detail page per hit, the site timed out). Each source has the 12 s timeout; a source that answers 403/429/503 is not asked again for 10 minutes (status `blocked`, "refused a moment ago"). Anime sources only run for anime titles (TMDB animation with original language ja/zh/ko; they are asked with the romaji title from TMDB's alternative titles, an episode as "Title 05") or for `GET /api/torrents/search?q=…&anime=1`.

```ts
// TorrentResult plus: seeders_unknown: boolean   // no source of this torrent reports seeders: 0 = not known
// Release.languages also takes languages a source states besides the name (Comet flags, MediaFusion and Zilean lists)
// facets.language also has "dl" = German plus the original ("German DL"); filter with language=dl
interface SourceInfo { id: string; imdb: boolean; anime: boolean; default_on: boolean; seeders: boolean; on: boolean }
// TorrentSettings plus:
//   sources_off: string[]   // default: torrentkitty, rutor, dmhy, mikan
//   allow_direct: boolean   // torrents may run without VPN (default false)
//   direct: boolean         // without VPN even when a VPN file is there (needs allow_direct)
// Vpn plus: route: "vpn" | "direct" | "none"; direct: boolean (the open route is the normal network); direct_allowed: boolean
```

| Method | Path | Returns |
|---|---|---|
| GET | `/api/torrents/sources` | `SourceInfo[]` in status-list order |
| PUT | `/api/torrents/settings` | also `sources_off` (unknown ids → `400`; a change empties the 5-minute search cache), `allow_direct`, `direct` (`direct` without `allow_direct` → `400`) |
| POST | `/api/torrents/start` | `409 { code: "needs_vpn" }` when `Vpn.route` is `"none"`: no VPN file and torrents without VPN not allowed |

Ranking: a torrent with unknown seeders gets −1500 instead of −2000 for "no seeders", so it sorts below every live swarm and above dead ones.

**Torrents without VPN (route).** `route` follows the settings: a VPN file and `direct` off → `"vpn"`, exactly as in Phase 5/6 (kill-switch, no fallback: a tunnel that cannot start or drops never turns into a direct route). `allow_direct` on and (`direct` on or no VPN file) → `"direct"`: the same SOCKS server, tracker client and DHT client use the PC's normal network instead of the tunnel; PEX as usual; nothing listens for incoming peers (no forwarded port); seeding stays off after a finished download and the upload limit stays. No VPN file and `allow_direct` off → `"none"`: nothing starts. Changing the route closes the current one; the next active torrent opens the new one. The UI asks for a one-time confirmation before `allow_direct` (upload while loading, real IP visible, legal risk) and shows "Ohne VPN" in the warning colour wherever the VPN state is shown.

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

All POST/PUT bodies above must be `application/json`. What keeps another web page out is the guard described under "Access, sessions and the phone mode".

**Folders.** Development (`--root`): data in `<root>/data` as before. Installed: resources next to `Flimmer.exe`,
data in `%LOCALAPPDATA%\local.flimmer.desktop\data` (the folder the uninstaller offers to delete). `--data <dir>`
overrides it. The desktop app uses the port of its last start, else 43300, else the next free one up to 43399
(`--port` pins one), and refuses a second start on the same data folder.

## Access, sessions and the phone mode ("Handy & Tablet im WLAN")

Threats, fixes and what stays open: [security.md](security.md). Every request passes one guard:

- `Host` must be `127.0.0.1:<port>` or `localhost:<port>` (LAN listener: `<lan address>:<port>`), else `403 { code: "forbidden_host" }`.
- `/api`: `Sec-Fetch-Site` (when sent) must be `same-origin` or `none`, `Origin` (when sent) must be the server's own address, else `403` (`cross_site` / `cross_origin`). The server sends no CORS headers.
- `/api` needs a session: the desktop window's cookie (role `pc`) or a paired device's cookie (role `device`). Without one: `401 { code: "pairing_required" }`. Open to everybody: `GET /api/session`, `POST /api/pair`, `GET /api/auth/enter`, and everything outside `/api` (the app shell).
- A device that calls something only the PC may do gets `403 { code: "pc_only" }` (list in security.md).
- Expensive endpoints have a budget per caller: `429 { code: "rate_limited" }` with `Retry-After`.
- Paths with `..`, `\`, NUL or double encoding: `400 { code: "bad_path" }`.

```ts
interface Session { role: "pc" | "device" | "none"; via: "local" | "lan"; device: string | null }   // device = its name in the PC's list
interface LanState {
  enabled: boolean;
  ip: string | null;                 // address picked by hand, null = automatic (the PC's main private address)
  active_ip: string | null;          // what the server listens on right now
  url: string | null;                // http://<active_ip>:<port>
  port: number;
  candidates: { ip: string; name: string }[];   // this PC's private IPv4 addresses
  error: string | null;              // why the address could not be opened
  pairing: { pin: string; link: string | null; qr: string | null; expires_in: number } | null;   // qr = SVG markup, link = <url>/pair#t=<one-time token>
  devices: { id: string; name: string; created_at: string; last_seen: string; last_ip: string }[];
}
```

| Method | Path | Answer |
|---|---|---|
| GET | `/api/auth/enter?t=<ticket>` | loopback only. A ticket from `Server::entry_url` works once: sets the session cookie (`HttpOnly; SameSite=Strict`) and redirects to `/`. Else `403`. The desktop window starts on this address; `flimmer-server` prints such a link (or opens it with `--open`). |
| GET | `/api/session` | `Session` |
| POST | `/api/pair` | JSON `{ pin }` or `{ token }` → `200 { ok: true }` and the device cookie (`HttpOnly; SameSite=Strict`, one year). Errors: `403 wrong_pin`, `409 closed` (no pairing open on the PC), `409 full` (20 devices), `429 locked` with `retry_after` seconds (5 wrong tries from one address; 15 wrong tries in total close the pairing) |
| POST | `/api/pair/logout` | `204`; the calling device removes itself |
| GET | `/api/lan` | PC only. `LanState` |
| PUT | `/api/lan` | PC only. JSON `{ enabled, ip? }` → `204`. `ip` must be one of `candidates`. The listener opens or closes within a moment; it never binds `0.0.0.0`, and it follows an address change within 15 s |
| POST | `/api/lan/pairing` | PC only. Opens a pairing for 10 minutes (new PIN and QR token, each good for one device) → `LanState`. `400` while the phone mode is off |
| DELETE | `/api/lan/pairing` | PC only. `204`; PIN and token stop working |
| DELETE | `/api/lan/devices/:id` | PC only. `204`; that device's token stops working at once |

**Stream proxy.** `/api/stream/:id/r` needs `&s=<signature>` next to `u`: the server signs every address it writes into a playlist (and the redirect of an mp4 stream) with stream id and address, so `r` only fetches what belongs to that stream (`403` otherwise). Addresses are never built by the UI. Outgoing requests go to public addresses only; a blocked one answers `403`.

**Folders.** `movies_dir`, `series_dir` and `vpn_config` must be paths on a local drive (`400` for `\server\share`, device paths, relative paths). `POST /api/torrents/:id/open` and `DELETE /api/torrents/:id?files=1` only act inside the download folders (`403` / files are kept).

**Import.** `version` must be known (`400`), the body is at most 1 MB (`413`), the watchlist at most 5000 titles; image addresses that are not TMDB's are dropped.

**PWA.** `/manifest.webmanifest` and `/sw.js` come with the web UI. The service worker keeps the app shell only and never answers `/api`. It is registered for paired devices where the browser allows one (https or localhost).

**Development.** `flimmer-server --dev` also accepts the Vite dev server (`localhost:5173`, which proxies `/api`) as host and origin; open the printed start link through it (`http://localhost:5173/api/auth/enter?t=…`).
