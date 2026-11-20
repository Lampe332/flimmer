# Flimmer API (Phase 1)

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

Everything not under `/api/` serves the built web UI (`web/dist`), unknown paths fall back to `index.html` (SPA routing).
