import type { AudioLang, Filter, ScanResult, Track, Variant } from './api'

const CODES: Record<Exclude<AudioLang, 'any'>, string[]> = {
  de: ['de', 'ger', 'deu'],
  en: ['en', 'eng'],
}
const NAMES: Record<Exclude<AudioLang, 'any'>, string[]> = {
  de: ['deutsch', 'german'],
  en: ['english', 'englisch'],
}

export function hasLanguage(tracks: Track[], want: AudioLang): boolean {
  if (want === 'any') return true
  return tracks.some((t) => {
    const lang = (t.lang ?? '').toLowerCase().split('-')[0] ?? ''
    const name = (t.name ?? '').toLowerCase()
    return CODES[want].includes(lang) || NAMES[want].some((n) => name.includes(n))
  })
}

export type Fit = { height: boolean; lang: boolean }

export function fit(r: ScanResult, f: Filter): Fit {
  return {
    height: f.min_height === 0 || (r.max_height ?? 0) >= f.min_height,
    lang: hasLanguage(r.audio, f.audio_lang),
  }
}

/** Playable results, best first: filter matches win, the sort setting breaks ties. */
export function rank(results: ScanResult[], f: Filter): ScanResult[] {
  const h = (r: ScanResult) => r.max_height ?? 0
  const b = (r: ScanResult) => r.max_bandwidth ?? 0
  const ms = (r: ScanResult) => r.ms ?? Number.MAX_SAFE_INTEGER
  const tie: Record<Filter['sort'], (a: ScanResult, z: ScanResult) => number> = {
    quality: (a, z) => h(z) - h(a) || b(z) - b(a) || ms(a) - ms(z),
    bitrate: (a, z) => b(z) - b(a) || h(z) - h(a) || ms(a) - ms(z),
    speed: (a, z) => ms(a) - ms(z) || h(z) - h(a) || b(z) - b(a),
  }
  return results
    .filter((r) => r.status === 'ok' && r.stream_id)
    .sort((a, z) => {
      const fa = fit(a, f)
      const fz = fit(z, f)
      return Number(fz.height) - Number(fa.height) || Number(fz.lang) - Number(fa.lang) || tie[f.sort](a, z)
    })
}

/** Seeders a torrent needs before the automatic choice trusts it to stream smoothly. */
export const TORRENT_MIN_SEEDS = 10
/** A resolution win alone needs a well seeded torrent. */
export const TORRENT_HEIGHT_SEEDS = 25

/**
 * Auto-pick rule between the best provider stream and the best torrent (both already ranked by the saved filter).
 * Providers stay first: they start in seconds and leave nothing on disk. The torrent only wins when it clearly
 * beats them for this filter AND is healthy (at least TORRENT_MIN_SEEDS seeders, meets the minimum height, no CAM):
 *   1. language: an audio language is wanted, the torrent has it and the best provider does not;
 *   2. resolution: the torrent is at least one step sharper (e.g. 1080p over 720p), has TORRENT_HEIGHT_SEEDS
 *      seeders, and is not worse on the wanted language than the provider.
 * Without any playable provider the torrent is only suggested, never started on its own: starting a torrent opens
 * the VPN tunnel and writes the file to disk, so that stays a click.
 */
export function torrentBeatsProvider(
  provider: ScanResult | undefined,
  torrent: { height: number | null; seeders: number; belowMin: boolean; cam: boolean; hasLang: boolean } | undefined,
  f: Filter,
): boolean {
  if (!provider || !torrent) return false
  if (torrent.seeders < TORRENT_MIN_SEEDS || torrent.belowMin || torrent.cam) return false
  const pFit = fit(provider, f)
  if (f.audio_lang !== 'any' && torrent.hasLang && !pFit.lang) return true
  // An unknown provider resolution is no reason to switch.
  const sharper = provider.max_height !== null && (torrent.height ?? 0) > provider.max_height && (torrent.height ?? 0) >= 720
  const langOk = f.audio_lang === 'any' || torrent.hasLang || !pFit.lang
  return sharper && langOk && torrent.seeders >= TORRENT_HEIGHT_SEEDS
}

/** Index into the server's variant list (best first) for a level the player reports. */
export function variantIndex(variants: Variant[], height: number, bitrate: number): number {
  const exact = variants.findIndex((v) => v.height === height && v.bandwidth === bitrate)
  if (exact >= 0) return exact
  const byHeight = variants.findIndex((v) => v.height === height)
  return byHeight >= 0 ? byHeight : 0
}
