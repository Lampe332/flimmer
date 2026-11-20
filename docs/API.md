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

Everything not under `/api/` serves the built web UI (`web/dist`), unknown paths fall back to `index.html` (SPA routing).
