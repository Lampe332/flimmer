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

export interface Source {
  id: string
  name: string
  no_ads: boolean
  url: string
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

export type ListKind = 'popular' | 'top_rated' | 'trending' | 'now_playing' | 'on_the_air'

/** Error with HTTP status; status 0 = backend unreachable. */
export class ApiError extends Error {
  readonly status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let res: Response
  try {
    res = await fetch(`/api${path}`, init)
  } catch {
    throw new ApiError(0, 'Der Flimmer-Server ist nicht erreichbar.')
  }
  if (!res.ok) {
    let message = `Fehler ${res.status}`
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
  sources: (type: MediaType, id: number, s?: number, e?: number) => {
    const qs = type === 'tv' ? `?s=${s ?? 1}&e=${e ?? 1}` : ''
    return request<Source[]>(`/sources/${type}/${id}${qs}`)
  },
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

export function playPath(type: MediaType, id: number, s?: number, e?: number): string {
  return type === 'tv' ? `/play/tv/${id}?s=${s ?? 1}&e=${e ?? 1}` : `/play/movie/${id}`
}
