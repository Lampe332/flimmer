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

/** Index into the server's variant list (best first) for a level the player reports. */
export function variantIndex(variants: Variant[], height: number, bitrate: number): number {
  const exact = variants.findIndex((v) => v.height === height && v.bandwidth === bitrate)
  if (exact >= 0) return exact
  const byHeight = variants.findIndex((v) => v.height === height)
  return byHeight >= 0 ? byHeight : 0
}
