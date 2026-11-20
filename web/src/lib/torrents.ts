import { useQuery, type QueryClient } from '@tanstack/react-query'
import { t, type Key } from '../i18n'
import { request, type AudioLang, type Filter, type MediaType } from './api'
import { locale } from './format'

/** Parsed from the release name (see docs/API.md, Phase 5). */
export interface Release {
  title: string
  year: number | null
  season: number | null
  episode: number | null
  pack: boolean
  height: 480 | 576 | 720 | 1080 | 2160 | null
  source: 'Remux' | 'BluRay' | 'WEB-DL' | 'WEBRip' | 'HDTV' | 'DVD' | 'CAM' | null
  codec: 'HEVC' | 'H.264' | 'AV1' | 'XviD' | null
  hdr: 'DV' | 'HDR10+' | 'HDR' | null
  audio: string | null
  languages: string[]
}

export interface TorrentResult {
  infohash: string
  name: string
  size: number
  seeders: number
  seeders_max: number
  leechers: number
  sources: string[]
  trackers: string[]
  file_index: number | null
  exact: boolean
  release: Release
  below_min: boolean
  score: number
  magnet: string
}

export interface SourceStatus {
  id: string
  status: 'ok' | 'blocked' | 'error' | 'skipped'
  count: number
  ms: number
  error: string | null
}

export interface TorrentSearch {
  results: TorrentResult[]
  sources: SourceStatus[]
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
}

export type TorrentState = 'metadata' | 'starting' | 'checking' | 'downloading' | 'paused' | 'completed' | 'error' | 'vpn_blocked'

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
}

export interface TorrentSettings {
  vpn_config: string
  movies_dir: string
  series_dir: string
  upload_kbps: number
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
}

const json = (method: string, body: unknown): RequestInit => ({
  method,
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(body),
})
const enc = encodeURIComponent

export const torrentApi = {
  search: (q: string) => request<TorrentSearch>(`/torrents/search?${new URLSearchParams({ q })}`),
  forTitle: (type: MediaType, id: number, s?: number, e?: number) =>
    request<TorrentSearch>(`/torrents/${type}/${id}${type === 'tv' ? `?s=${s ?? 1}&e=${e ?? 1}` : ''}`),
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

export function formatSpeed(bps: number): string {
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
  return mp4 && r.release.codec !== 'XviD' && !['DTS', 'DTS-HD', 'TrueHD', 'AC3', 'EAC3'].includes(r.release.audio ?? '')
}

// ---- VPN in plain words ----

export type VpnProblem = { title: Key; text: Key }

/** A friendly explanation for why the tunnel does not work, or null when it is fine. */
export function vpnProblem(v: Vpn | undefined): VpnProblem | null {
  if (!v) return null
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
