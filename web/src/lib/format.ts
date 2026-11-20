import { getLang, localeOf, t, type Key } from '../i18n'

const dateFmts = new Map<string, Intl.DateTimeFormat>()

/** Intl formats in the chosen UI language. */
export function locale(): string {
  return localeOf(getLang())
}

export function formatDate(iso: string | null): string | null {
  if (!iso) return null
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return null
  const loc = locale()
  let fmt = dateFmts.get(loc)
  if (!fmt) {
    fmt = new Intl.DateTimeFormat(loc, { day: 'numeric', month: 'short', year: 'numeric' })
    dateFmts.set(loc, fmt)
  }
  return fmt.format(d)
}

export function formatRuntime(min: number | null): string | null {
  if (!min) return null
  const h = Math.floor(min / 60)
  const m = min % 60
  return h > 0 ? t('fmt.hm', { h, m }) : t('fmt.m', { m })
}

export function formatRating(r: number | null): string | null {
  return r === null ? null : r.toLocaleString(locale(), { minimumFractionDigits: 1, maximumFractionDigits: 1 })
}

export function typeLabel(type: 'movie' | 'tv'): string {
  return t(type === 'movie' ? 'type.movie' : 'type.tv')
}

/** Episode has not aired yet (air date in the future). */
export function isUpcoming(iso: string | null): boolean {
  if (!iso) return false
  const d = new Date(iso)
  return !Number.isNaN(d.getTime()) && d.getTime() > Date.now()
}

/** Playback clock: 1:02:03 or 2:03. */
export function formatClock(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '0:00'
  const s = Math.floor(seconds)
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const rest = String(s % 60).padStart(2, '0')
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${rest}` : `${m}:${rest}`
}

export function formatHeight(height: number | null): string | null {
  if (!height) return null
  return height >= 2160 ? '4K' : `${height}p`
}

export function formatBitrate(bps: number | null): string | null {
  if (!bps) return null
  const mbit = bps / 1_000_000
  return mbit >= 1
    ? `${mbit.toLocaleString(locale(), { maximumFractionDigits: 1 })} Mbit/s`
    : `${Math.round(bps / 1000)} kbit/s`
}

export function formatMs(ms: number | null): string | null {
  if (ms === null) return null
  return `${(ms / 1000).toLocaleString(locale(), { minimumFractionDigits: 1, maximumFractionDigits: 1 })} s`
}

/** Playlist language codes and English names, each mapped to its text key. */
const LANGS: [Key, string[], string][] = [
  ['lang.de', ['de', 'ger', 'deu'], 'german'],
  ['lang.en', ['en', 'eng'], 'english'],
  ['lang.fr', ['fr', 'fre', 'fra'], 'french'],
  ['lang.es', ['es', 'spa'], 'spanish'],
  ['lang.it', ['it', 'ita'], 'italian'],
  ['lang.ja', ['ja', 'jpn'], 'japanese'],
  ['lang.ko', ['ko', 'kor'], 'korean'],
  ['lang.tr', ['tr', 'tur'], 'turkish'],
  ['lang.pt', ['pt', 'por'], 'portuguese'],
  ['lang.ru', ['ru', 'rus'], 'russian'],
]
/** Playlist tags in track names, in plain words. */
const TAGS: [RegExp, Key][] = [
  [/\s*[[(]\s*forced\s*[\])]/i, 'track.forced'],
  [/\s*[[(]\s*(cc|sdh)\s*[\])]/i, 'track.cc'],
]

/** Readable name for a playlist track: "Italian [Forced]" → "Italienisch (nur Fremdsprache)". */
export function trackLabel(track: { lang?: string | null; name?: string | null }, index: number, kind: string): string {
  const code = track.lang?.toLowerCase()
  const known = code ? LANGS.find(([, codes]) => codes.includes(code)) : undefined
  let name = track.name?.trim() ?? ''
  if (known && name.length <= 3) return t(known[0])
  for (const [key, , english] of LANGS) name = name.replace(new RegExp(`^${english}\\b`, 'i'), t(key))
  for (const [re, key] of TAGS) name = name.replace(re, t(key))
  return name || (track.lang && track.lang !== 'und' ? track.lang.toUpperCase() : `${kind} ${index + 1}`)
}

/** Short language code for badges: DE, EN, or the raw code. */
export function langCode(lang: string | null): string | null {
  if (!lang || lang === 'und') return null
  const l = lang.toLowerCase()
  if (['de', 'ger', 'deu'].includes(l)) return 'DE'
  if (['en', 'eng'].includes(l)) return 'EN'
  return l.slice(0, 3).toUpperCase()
}
