import { useQuery, type QueryClient } from '@tanstack/react-query'
import { t, type Key } from '../i18n'
import { request, type AudioLang, type Filter, type MediaType } from './api'
import { locale } from './format'
import { FILTER_DIMS, type FilterDim, type TorrentFilter, type TorrentMode } from './prefs'

/** Parsed from the release name (see docs/API.md, Phases 5 and 6). */
export interface Release {
  title: string
  year: number | null
  season: number | null
  episode: number | null
  pack: boolean
  height: number | null
  source: string | null
  cam: boolean
  codec: string | null
  bit_depth: number | null
  hdr: string | null
  hdr_all: string[]
  audio: string | null
  channels: string | null
  languages: string[]
  subtitles: string[]
  dual: boolean
  dub: string | null
  group: string | null
  proper: boolean
  repack: boolean
  edition: string | null
  '3d': boolean
}

export interface TorrentResult {
  infohash: string
  name: string
  size: number
  seeders: number
  seeders_max: number
  /** No source reports seeders for it: 0 means "not known". */
  seeders_unknown: boolean
  leechers: number
  sources: string[]
  trackers: string[]
  file_index: number | null
  exact: boolean
  added: number | null
  age_days: number | null
  release: Release
  below_min: boolean
  score: number
  magnet: string
}

export interface SourceStatus {
  id: string
  status: 'ok' | 'blocked' | 'error' | 'skipped' | 'off'
  count: number
  ms: number
  error: string | null
}

export type Facets = Partial<Record<FilterDim | 'cam', Record<string, number>>>

export interface TorrentSearch {
  results: TorrentResult[]
  sources: SourceStatus[]
  facets: Facets
  /** Merged results before the filters. */
  total: number
  /** After the filters (`results` holds the best 120). */
  matched: number
}

export interface SourceInfo {
  id: string
  imdb: boolean
  anime: boolean
  default_on: boolean
  seeders: boolean
  on: boolean
}

export interface SubtitleInfo {
  index: number
  codec: string
  lang: string | null
  title: string | null
  default: boolean
  forced: boolean
  /** Can be served as WebVTT (image subtitles cannot). */
  text: boolean
}

export interface NextFile {
  file: number
  season: number | null
  episode: number | null
}

export interface TorrentEvent {
  id: number
  at: string
  kind: 'finished' | 'failed' | 'grabbed'
  torrent_id: string
  name: string
  media: TorrentMedia | null
  message: string | null
}

export interface AutoRule {
  type: MediaType
  id: number
  title: string
  year: number | null
  enabled: boolean
  min_height: 0 | 480 | 720 | 1080 | 2160
  max_height: 0 | 480 | 720 | 1080 | 2160
  lang: string
  sources: string[]
  max_size_gb: number
  min_seeders: number
  since: string
  created_at: number
  on_watchlist: boolean
}

export type AutoRuleBody = Omit<AutoRule, 'type' | 'id' | 'created_at' | 'on_watchlist' | 'since'> & { since?: string }

interface Grab {
  key: string
  type: MediaType
  id: number
  title: string
  season: number | null
  episode: number | null
  infohash: string
  name: string
  size: number
  seeders: number
  at: string
}

export interface Decision {
  rule: string
  title: string
  season: number | null
  episode: number | null
  air_date: string | null
  action: 'grab' | 'none' | 'skip'
  reason: string | null
  found: number
  candidate: { infohash: string; name: string; size: number; seeders: number; height: number | null; source: string | null; languages: string[] } | null
}

export interface Vpn {
  state: 'down' | 'connecting' | 'up' | 'blocked'
  since: string | null
  public_ip: string | null
  country: string | null
  handshake_age: number | null
  rx_bytes: number
  tx_bytes: number
  error: string | null
  config_found: boolean
  /** How torrents go with the current settings: VPN, the normal network (allowed by the user) or not at all. */
  route: 'vpn' | 'direct' | 'none'
  /** The open route is the normal network. */
  direct: boolean
  direct_allowed: boolean
}

export interface TorrentMedia {
  type: MediaType
  id?: number
  title: string
  year?: number
  season?: number
  episode?: number
}

export interface TorrentFile {
  index: number
  name: string
  size: number
  path: string
  selected: boolean
  downloaded: number
  video: boolean
  season: number | null
  episode: number | null
}

export type TorrentState = 'metadata' | 'starting' | 'checking' | 'downloading' | 'queued' | 'paused' | 'completed' | 'error' | 'vpn_blocked'

export interface Torrent {
  id: string
  name: string
  mode: 'stream' | 'download'
  media: TorrentMedia | null
  state: TorrentState
  progress: number
  downloaded: number
  total: number
  down_speed: number
  up_speed: number
  peers: number
  peers_known: number
  eta_seconds: number | null
  folder: string
  error: string | null
  added_at: string
  files: TorrentFile[]
  /** -100..100; the queue runs streams first, then priority, then age. */
  priority: number
  queue_position: number | null
  /** Queued by an auto-download rule. */
  auto: boolean
}

export interface AudioInfo {
  index: number
  codec: string
  channels: number
  lang: string | null
  title: string | null
  default: boolean
}

export interface PlayInfo {
  file: number
  name: string
  size: number
  mode: 'direct' | 'transcode'
  url: string
  duration: number | null
  video_codec: string | null
  width: number | null
  height: number | null
  copy_video: boolean
  audio: AudioInfo[]
  /** The saved audio language's track, else the file's default. */
  default_audio: number
  hevc: boolean
  subtitles: SubtitleInfo[]
  next_file: NextFile | null
}

export interface TorrentSettings {
  vpn_config: string
  movies_dir: string
  series_dir: string
  upload_kbps: number
  max_active: number
  min_free_gb: number
  sources_off: string[]
  /** Torrents may run without VPN (confirmed warning). */
  allow_direct: boolean
  /** Without VPN even when a VPN file is there. */
  direct: boolean
}

export interface StartBody {
  magnet?: string
  infohash?: string
  trackers?: string[]
  name?: string
  mode?: 'stream' | 'download'
  file_index?: number
  files?: number[]
  media?: TorrentMedia
  priority?: number
}

/** Filter + order as query parameters (`language`, not `lang`: that one is the TMDB language). */
export function filterParams(f: TorrentFilter, mode: TorrentMode): URLSearchParams {
  const p = new URLSearchParams()
  for (const dim of FILTER_DIMS) {
    const list = f.values[dim]
    if (list && list.length > 0) p.set(dim, list.join(','))
  }
  if (f.minSeeders > 0) p.set('min_seeders', String(f.minSeeders))
  if (f.minSizeGb > 0) p.set('min_size', String(Math.round(f.minSizeGb * 1e9)))
  if (f.maxSizeGb > 0) p.set('max_size', String(Math.round(f.maxSizeGb * 1e9)))
  if (f.cam) p.set('exclude_cam', 'false')
  p.set('mode', mode)
  return p
}

const withParams = (path: string, p?: URLSearchParams) => {
  const q = p?.toString()
  return q ? `${path}${path.includes('?') ? '&' : '?'}${q}` : path
}

const json = (method: string, body: unknown): RequestInit => ({
  method,
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(body),
})
const enc = encodeURIComponent

export const torrentApi = {
  search: (q: string, p?: URLSearchParams, anime?: boolean) =>
    request<TorrentSearch>(withParams(`/torrents/search?${new URLSearchParams({ q })}${anime ? '&anime=1' : ''}`, p)),
  forTitle: (type: MediaType, id: number, s?: number, e?: number, p?: URLSearchParams) =>
    request<TorrentSearch>(withParams(`/torrents/${type}/${id}${type === 'tv' ? `?s=${s ?? 1}&e=${e ?? 1}` : ''}`, p)),
  sources: () => request<SourceInfo[]>('/torrents/sources'),
  priority: (id: string, priority: number) => request<void>(`/torrents/${enc(id)}/priority`, json('PUT', { priority })),
  events: (since: number) => request<{ events: TorrentEvent[]; last: number }>(`/torrents/events?since=${since}`),
  seek: (id: string, file: number, t: number, h264: boolean) =>
    request<{ t: number; start: number; requested: number; exact: boolean }>(
      `/torrents/${enc(id)}/seek?file=${file}&t=${t.toFixed(2)}${h264 ? '&video=h264' : ''}`,
    ),
  next: (id: string, file: number) => request<{ file: number; season: number | null; episode: number | null }>(`/torrents/${enc(id)}/next?file=${file}`),
  rules: () => request<{ rules: AutoRule[]; last_run: string | null; interval_minutes: number }>('/autodl/rules'),
  rulePut: (type: MediaType, id: number, body: AutoRuleBody) => request<void>(`/autodl/rules/${type}/${id}`, json('PUT', body)),
  ruleDelete: (type: MediaType, id: number) => request<void>(`/autodl/rules/${type}/${id}`, { method: 'DELETE' }),
  rulePreview: (type: MediaType, id: number, body: AutoRuleBody) =>
    request<{ dry: boolean; decisions: Decision[] }>(`/autodl/preview/${type}/${id}`, json('POST', body)),
  grabs: () => request<Grab[]>('/autodl/history'),
  grabDelete: (key: string) => request<void>(`/autodl/history/${enc(key)}`, { method: 'DELETE' }),
  start: (body: StartBody) => request<Torrent>('/torrents/start', json('POST', body)),
  list: () => request<{ vpn: Vpn; torrents: Torrent[] }>('/torrents'),
  get: (id: string) => request<Torrent>(`/torrents/${enc(id)}`),
  pause: (id: string) => request<void>(`/torrents/${enc(id)}/pause`, { method: 'POST', keepalive: true }),
  resume: (id: string) => request<void>(`/torrents/${enc(id)}/resume`, { method: 'POST' }),
  files: (id: string, files: number[]) => request<void>(`/torrents/${enc(id)}/files`, json('PUT', { files })),
  remove: (id: string, withFiles: boolean) => request<void>(`/torrents/${enc(id)}${withFiles ? '?files=1' : ''}`, { method: 'DELETE' }),
  open: (id: string) => request<void>(`/torrents/${enc(id)}/open`, { method: 'POST' }),
  playinfo: (id: string, file?: number) => request<PlayInfo>(`/torrents/${enc(id)}/playinfo${file === undefined ? '' : `?file=${file}`}`),
  settings: () => request<TorrentSettings>('/torrents/settings'),
  settingsPut: (s: TorrentSettings) => request<void>('/torrents/settings', json('PUT', s)),
  /** A form save: source switches and "without VPN" are saved on their own, so the server's values are kept. */
  settingsSave: async (s: TorrentSettings) => {
    const current = await torrentApi.settings()
    await torrentApi.settingsPut({ ...s, sources_off: current.sources_off, allow_direct: current.allow_direct, direct: current.direct })
  },
  vpn: () => request<Vpn>('/vpn'),
  vpnRelease: () => request<void>('/vpn/release', { method: 'POST' }),
}

/** The torrent list; polls quickly while something loads, slowly otherwise (the nav badge needs it). */
export function useTorrents() {
  return useQuery({
    queryKey: ['torrents'],
    queryFn: torrentApi.list,
    refetchInterval: (q) => (q.state.data?.torrents.some(isActive) ? 1500 : 8000),
    retry: false,
  })
}

export function refreshTorrents(qc: QueryClient) {
  void qc.invalidateQueries({ queryKey: ['torrents'] })
}

export const isActive = (x: Torrent) => ['metadata', 'starting', 'checking', 'downloading'].includes(x.state)

/** Address of the playable stream of a torrent file; `t` and `audio` switch the server to ffmpeg. */
export function playUrl(id: string, file: number, opts: { t?: number; audio?: number | null; h264?: boolean } = {}): string {
  const p = new URLSearchParams({ file: String(file) })
  if (opts.t && opts.t > 0) p.set('t', opts.t.toFixed(1))
  if (opts.audio !== undefined && opts.audio !== null) p.set('audio', String(opts.audio))
  if (opts.h264) p.set('video', 'h264')
  return `/api/torrents/${enc(id)}/play?${p}`
}

/** A text subtitle track as WebVTT; `start` = file time of the stream's first frame (cues move with it). */
export function subtitleUrl(id: string, file: number, track: number, start: number): string {
  return `/api/torrents/${enc(id)}/subtitle?file=${file}&track=${track}${start > 0 ? `&t=${start.toFixed(3)}` : ''}`
}

export function torrentPlayPath(id: string, file?: number): string {
  return `/play/torrent/${id}${file === undefined ? '' : `?file=${file}`}`
}

// ---- formatting ----

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB']

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0 MB'
  let i = 0
  let v = n
  while (v >= 1000 && i < UNITS.length - 1) {
    v /= 1000
    i += 1
  }
  const digits = v >= 100 || i < 2 ? 0 : 1
  return `${v.toLocaleString(locale(), { maximumFractionDigits: digits, minimumFractionDigits: digits })} ${UNITS[i]}`
}

function formatSpeed(bps: number): string {
  return `${formatBytes(bps)}/s`
}

export function formatEta(seconds: number | null): string | null {
  if (seconds === null || !Number.isFinite(seconds) || seconds <= 0) return null
  if (seconds < 60) return t('tor.etaS', { n: Math.ceil(seconds) })
  const m = Math.round(seconds / 60)
  if (m < 60) return t('tor.etaM', { n: m })
  return t('tor.etaH', { h: Math.floor(m / 60), m: m % 60 })
}

/** Search sources, as their sites call themselves. */
const SOURCE_NAMES: Record<string, string> = {
  torrentio: 'Torrentio',
  knaben: 'Knaben',
  yts: 'YTS',
  eztv: 'EZTV',
  nyaa: 'Nyaa',
  piratebay: 'The Pirate Bay',
  '1337x': '1337x',
  therarbg: 'TheRARBG',
  limetorrents: 'LimeTorrents',
  comet: 'Comet',
  cometweebs: 'Comet (Weebs)',
  torrentsdb: 'TorrentsDB',
  mediafusion: 'MediaFusion',
  peerflix: 'Peerflix',
  bitsearch: 'BitSearch',
  torrentscsv: 'Torrents-CSV',
  torrentdownloads: 'TorrentDownloads',
  torrentdownload: 'TorrentDownload.info',
  zilean: 'Zilean',
  bt4g: 'BT4G',
  torrentkitty: 'TorrentKitty',
  rutor: 'Rutor',
  subsplease: 'SubsPlease',
  animetosho: 'AnimeTosho',
  tokyotosho: 'TokyoTosho',
  dmhy: 'DMHY',
  mikan: 'Mikan',
}
export const sourceName = (id: string) => SOURCE_NAMES[id] ?? id

// ---- languages and fit ----

/** Audio chips for a release: "DL" (German + English) and "MULTi" stay as the scene writes them. */
export function audioChips(r: Release): { label: string; strong: boolean }[] {
  const langs = r.languages
  const de = langs.includes('de')
  const out: { label: string; strong: boolean }[] = []
  if (de && langs.includes('en')) out.push({ label: 'DL', strong: true })
  else if (de) out.push({ label: 'DE', strong: true })
  if (langs.includes('multi')) out.push({ label: 'MULTi', strong: true })
  for (const l of langs) {
    if (l === 'de' || l === 'multi' || (l === 'en' && de)) continue
    out.push({ label: l.toUpperCase(), strong: false })
  }
  return out
}

/** Release states the wanted audio language. "MULTi" counts only for German (it nearly always carries it). */
export function torrentHasLang(r: Release, want: AudioLang): boolean {
  if (want === 'any') return true
  if (r.languages.includes(want)) return true
  if (want === 'de') return r.languages.includes('multi')
  // English is what an unlabelled release usually is.
  return r.languages.length === 0 || r.languages.includes('multi')
}

export function torrentFit(r: TorrentResult, f: Filter): { height: boolean; lang: boolean } {
  return { height: !r.below_min, lang: torrentHasLang(r.release, f.audio_lang) }
}

/** Server order (its score already weighs language, sort and seeders), with filter misses moved down. */
export function rankTorrents(results: TorrentResult[], f: Filter): TorrentResult[] {
  return results
    .map((r, i) => ({ r, i, fit: torrentFit(r, f) }))
    .sort((a, z) => Number(z.fit.height) - Number(a.fit.height) || Number(z.fit.lang) - Number(a.fit.lang) || a.i - z.i)
    .map((x) => x.r)
}

/**
 * Likely to play in the browser without ffmpeg: an MP4 with H.264/HEVC/AV1. YTS ships exactly that.
 * Only a hint: the real decision comes from /playinfo once the file's start is there.
 */
export function likelyDirect(r: TorrentResult): boolean {
  const mp4 = /\bmp4\b|\.mp4$/i.test(r.name) || r.sources.includes('yts')
  return mp4 && r.release.codec !== 'XviD' && !['DTS', 'DTS-HD', 'DTS-HD MA', 'TrueHD', 'AC3', 'EAC3'].includes(r.release.audio ?? '')
}

// ---- VPN in plain words ----

type VpnProblem = { title: Key; text: Key }

/** A friendly explanation for why the tunnel does not work, or null when it is fine. */
export function vpnProblem(v: Vpn | undefined): VpnProblem | null {
  if (!v) return null
  // Without VPN, allowed by the user: no VPN file is not a problem then.
  if (v.route === 'direct' && v.state !== 'blocked') return null
  // No VPN file and "without VPN" not allowed: say both ways out.
  if (v.route === 'none') return { title: 'vpn.missingTitle', text: 'direct.needsVpn' }
  if (!v.config_found) return { title: 'vpn.missingTitle', text: 'vpn.missingText' }
  if (v.state === 'blocked') return { title: 'vpn.blockedTitle', text: 'vpn.blockedText' }
  if (v.state === 'up' || !v.error) return null
  if (/handshake|unreachable/i.test(v.error)) return { title: 'vpn.handshakeTitle', text: 'vpn.handshakeText' }
  if (/config/i.test(v.error)) return { title: 'vpn.missingTitle', text: 'vpn.badConfigText' }
  return { title: 'vpn.errorTitle', text: 'vpn.errorText' }
}

/** Short state line for a torrent: "Lädt von 34 Quellen · 12 MB/s". */
export function torrentLine(x: Torrent): string {
  switch (x.state) {
    case 'metadata':
      return t('tor.stMetadata')
    case 'starting':
      return t('tor.stStarting')
    case 'checking':
      return t('tor.stChecking')
    case 'queued':
      return x.queue_position ? t('tor.stQueuedAt', { n: x.queue_position }) : t('tor.stQueued')
    case 'paused':
      return t('tor.stPaused')
    case 'completed':
      return t('tor.stCompleted')
    case 'vpn_blocked':
      return t('tor.stVpn')
    case 'error':
      return t('tor.stError')
    default:
      return x.peers > 0
        ? t('tor.stLoading', { n: x.peers, speed: formatSpeed(x.down_speed) })
        : t('tor.stSearchingPeers')
  }
}

/** "Fight Club (1999)" / "Breaking Bad S01E02" / the torrent's own name. */
export function torrentTitle(x: Torrent): string {
  const m = x.media
  if (!m) return x.name
  if (m.type === 'tv' && m.season) {
    const code = m.episode ? `S${String(m.season).padStart(2, '0')}E${String(m.episode).padStart(2, '0')}` : t('tor.seasonN', { n: m.season })
    return `${m.title} · ${code}`
  }
  return m.year ? `${m.title} (${m.year})` : m.title
}

/** SxxEyy in a file name → episode number in that season. */
export function episodeOfFile(path: string): { s: number; e: number } | null {
  const m = /s(\d{1,2})[ ._-]?e(\d{1,3})/i.exec(path) ?? /\b(\d{1,2})x(\d{2,3})\b/i.exec(path)
  return m ? { s: Number(m[1]), e: Number(m[2]) } : null
}
