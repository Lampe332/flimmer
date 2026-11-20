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
