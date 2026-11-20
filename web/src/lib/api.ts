import { getLang, localeOf, t } from '../i18n'

export type MediaType = 'movie' | 'tv'

export interface Item {
  id: number
  type: MediaType
  title: string
  year: number | null
  poster: string | null
  backdrop: string | null
  rating: number | null
  overview: string
}

export interface Season {
  season_number: number
  name: string
  episode_count: number
  poster: string | null
  air_date: string | null
}

export interface CastMember {
  name: string
  character: string
  profile: string | null
}

export interface Details extends Item {
  original_title: string
  imdb_id: string | null
  genres: { id: number; name: string }[]
  runtime: number | null
  tagline: string | null
  trailer_youtube_key: string | null
  seasons: Season[]
  cast: CastMember[]
  recommendations: Item[]
}

export interface Episode {
  episode_number: number
  name: string
  overview: string
  still: string | null
  air_date: string | null
  runtime: number | null
}

export interface SeasonDetails {
  season_number: number
  name: string
  episodes: Episode[]
}

export interface Caps {
  sniffer: boolean
}

export interface Variant {
  width: number | null
  height: number | null
  bandwidth: number | null
  codecs: string | null
}

export interface Track {
  lang: string | null
  name: string | null
}

export type ScanStatus = 'pending' | 'running' | 'ok' | 'no_stream' | 'error'

export interface ScanResult {
  provider_id: string
  name: string
  status: ScanStatus
  ms: number | null
  max_width: number | null
  max_height: number | null
  max_bandwidth: number | null
  variants: Variant[]
  audio: Track[]
  subtitles: Track[]
  stream_id: string | null
  error: string | null
  /** "mp4" streams are a single file: played natively, no prefetch. */
  kind?: 'hls' | 'mp4'
}

export interface Scan {
  state: 'idle' | 'running' | 'done' | 'unavailable'
  started_at: string | null
  finished_at: string | null
  results: ScanResult[]
}

export type MinHeight = 0 | 480 | 720 | 1080 | 2160
export type AudioLang = 'any' | 'de' | 'en'
export type SortKey = 'quality' | 'bitrate' | 'speed'

export interface Filter {
  min_height: MinHeight
  audio_lang: AudioLang
  sort: SortKey
}

export interface Settings {
  filter: Filter
  prefetch_minutes: number
}

export interface BufferState {
  variant: number
  from_seconds: number
  ahead_seconds: number
  bytes: number
  done: boolean
}

export interface Page {
  results: Item[]
  page: number
  total_pages: number
}

export interface Home {
  trending: Item[]
  popular_movies: Item[]
  popular_tv: Item[]
  top_rated_movies: Item[]
  top_rated_tv: Item[]
}

export interface Genre {
  id: number
  name: string
}

/** Saved position of a movie or one episode. Times in seconds. */
export interface Progress {
  type: MediaType
  id: number
  season: number | null
  episode: number | null
  position: number
  duration: number
  finished: boolean
  updated_at: string
}

/** What a progress save / "watched" mark carries along so lists render without TMDB calls. */
export interface WatchMeta {
  item: Item
  episode_name?: string | null
  still?: string | null
}

export interface ContinueEntry {
  item: Item
  season: number | null
  episode: number | null
  episode_name: string | null
  still: string | null
  position: number
  duration: number
  /** Previous episode finished: this one is next and not started yet. */
  up_next: boolean
  updated_at: string
}

export interface HistoryEntry {
  entry: number
  item: Item
  season: number | null
  episode: number | null
  episode_name: string | null
  still: string | null
  position: number
  duration: number
  finished: boolean
  watched_at: string
}

export interface NewEpisode {
  item: Item
  season: number
  episode: number
  name: string
  still: string | null
  air_date: string
  count: number
}

export type ListKind = 'popular' | 'top_rated' | 'trending' | 'now_playing' | 'on_the_air'

/** Error with HTTP status; status 0 = backend unreachable. */
export class ApiError extends Error {
  readonly status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

/** Every call carries the UI language, so TMDB titles, overviews, episodes and genres come back in it. */
export async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const lang = `lang=${localeOf(getLang())}`
  let res: Response
  try {
    res = await fetch(`/api${path}${path.includes('?') ? '&' : '?'}${lang}`, init)
  } catch {
    throw new ApiError(0, t('api.offline'))
  }
  if (!res.ok) {
    let message = t('api.status', { status: res.status })
    try {
      const body = (await res.json()) as { error?: string }
      if (body.error) message = body.error
    } catch {
      /* body was not JSON, keep status message */
    }
    throw new ApiError(res.status, message)
  }
  if (res.status === 204) return undefined as T
  return (await res.json()) as T
}

export const api = {
  home: () => request<Home>('/home'),
  list: (type: MediaType, list: ListKind, page: number, genre: number | null) => {
    const params = new URLSearchParams({ page: String(page) })
    if (genre !== null) params.set('genre', String(genre))
    return request<Page>(`/list/${type}/${list}?${params}`)
  },
  genres: (type: MediaType) => request<Genre[]>(`/genres/${type}`),
  search: (q: string, page: number) =>
    request<Page>(`/search?${new URLSearchParams({ q, page: String(page) })}`),
  title: (type: MediaType, id: number) => request<Details>(`/title/${type}/${id}`),
  season: (id: number, n: number) => request<SeasonDetails>(`/title/tv/${id}/season/${n}`),
  caps: () => request<Caps>('/caps'),
  scan: (type: MediaType, id: number, s: number, e: number) =>
    request<Scan>(`/scan/${type}/${id}${episodeQuery(type, s, e)}`),
  /** Starts a scan (or returns the running / still fresh one). `provider` re-scans a single provider. */
  scanStart: (type: MediaType, id: number, s: number, e: number, opts: { force?: boolean; provider?: string } = {}) => {
    const params = new URLSearchParams(episodeQuery(type, s, e))
    if (opts.force) params.set('force', '1')
    if (opts.force && opts.provider) params.set('provider', opts.provider)
    const qs = params.toString()
    return request<Scan>(`/scan/${type}/${id}${qs ? `?${qs}` : ''}`, { method: 'POST' })
  },
  settings: () => request<Settings>('/settings'),
  settingsPut: (settings: Settings) =>
    request<void>('/settings', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(settings),
    }),
  prefetch: (streamId: string, variant: number, from: number) =>
    request<void>(
      `/stream/${encodeURIComponent(streamId)}/prefetch?variant=${variant}&from=${Math.max(0, Math.floor(from))}`,
      { method: 'POST' },
    ),
  buffer: (streamId: string) => request<BufferState>(`/stream/${encodeURIComponent(streamId)}/buffer`),
  /** Stops the prefetch of a stream and frees its cache; the stream itself stays playable. */
  streamRelease: (streamId: string) =>
    request<void>(`/stream/${encodeURIComponent(streamId)}`, { method: 'DELETE', keepalive: true }),
  progress: (type: MediaType, id: number) => request<Progress[]>(`/progress/${type}/${id}`),
  /** keepalive: also delivered when the page is being closed. */
  progressPut: (type: MediaType, id: number, s: number, e: number, body: WatchMeta & { position: number; duration: number }) =>
    request<void>(`/progress/${type}/${id}${episodeQuery(type, s, e)}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...body, item: toItem(body.item) }),
      keepalive: true,
    }),
  markWatched: (type: MediaType, id: number, s: number, e: number, meta: WatchMeta) =>
    request<void>(`/watched/${type}/${id}${episodeQuery(type, s, e)}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...meta, item: toItem(meta.item) }),
    }),
  markUnwatched: (type: MediaType, id: number, s: number, e: number) =>
    request<void>(`/watched/${type}/${id}${episodeQuery(type, s, e)}`, { method: 'DELETE' }),
  continueList: () => request<ContinueEntry[]>('/continue'),
  continueHide: (type: MediaType, id: number) => request<void>(`/continue/${type}/${id}`, { method: 'DELETE' }),
  history: () => request<HistoryEntry[]>('/history'),
  historyDelete: (entry: number) => request<void>(`/history/${entry}`, { method: 'DELETE' }),
  historyClear: () => request<void>('/history', { method: 'DELETE' }),
  newEpisodes: () => request<NewEpisode[]>('/new-episodes'),
  watchlist: () => request<Item[]>('/watchlist'),
  watchlistAdd: (item: Item) =>
    request<void>(`/watchlist/${item.type}/${item.id}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(toItem(item)),
    }),
  watchlistRemove: (type: MediaType, id: number) =>
    request<void>(`/watchlist/${type}/${id}`, { method: 'DELETE' }),
}

function episodeQuery(type: MediaType, s: number, e: number): string {
  return type === 'tv' ? `?s=${s}&e=${e}` : ''
}

export function streamUrl(streamId: string): string {
  return `/api/stream/${encodeURIComponent(streamId)}/master.m3u8`
}

/** Strip Details down to the plain Item shape the watchlist stores. */
export function toItem(x: Item): Item {
  return {
    id: x.id,
    type: x.type,
    title: x.title,
    year: x.year,
    poster: x.poster,
    backdrop: x.backdrop,
    rating: x.rating,
    overview: x.overview,
  }
}

export function isMediaType(v: string | undefined): v is MediaType {
  return v === 'movie' || v === 'tv'
}

export function titlePath(item: Pick<Item, 'type' | 'id'>): string {
  return `/titel/${item.type}/${item.id}`
}

/** "Play" without choosing an episode: a series continues where you left off (the player resolves it). */
export function playStartPath(item: Pick<Item, 'type' | 'id'>): string {
  return `/play/${item.type}/${item.id}`
}

export function playPath(type: MediaType, id: number, s?: number, e?: number): string {
  return type === 'tv' ? `/play/tv/${id}?s=${s ?? 1}&e=${e ?? 1}` : `/play/movie/${id}`
}
