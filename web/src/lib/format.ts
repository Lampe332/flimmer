const dateFmt = new Intl.DateTimeFormat('de-DE', { day: 'numeric', month: 'short', year: 'numeric' })

export function formatDate(iso: string | null): string | null {
  if (!iso) return null
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? null : dateFmt.format(d)
}

export function formatRuntime(min: number | null): string | null {
  if (!min) return null
  const h = Math.floor(min / 60)
  const m = min % 60
  return h > 0 ? `${h} Std. ${m} Min.` : `${m} Min.`
}

export function formatRating(r: number | null): string | null {
  return r === null ? null : r.toLocaleString('de-DE', { minimumFractionDigits: 1, maximumFractionDigits: 1 })
}

export function typeLabel(type: 'movie' | 'tv'): string {
  return type === 'movie' ? 'Film' : 'Serie'
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
  const t = Math.floor(seconds)
  const h = Math.floor(t / 3600)
  const m = Math.floor((t % 3600) / 60)
  const s = String(t % 60).padStart(2, '0')
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`
}

export function formatHeight(height: number | null): string | null {
  if (!height) return null
  return height >= 2160 ? '4K' : `${height}p`
}

export function formatBitrate(bps: number | null): string | null {
  if (!bps) return null
  const mbit = bps / 1_000_000
  return mbit >= 1
    ? `${mbit.toLocaleString('de-DE', { maximumFractionDigits: 1 })} Mbit/s`
    : `${Math.round(bps / 1000)} kbit/s`
}

export function formatMs(ms: number | null): string | null {
  if (ms === null) return null
  return `${(ms / 1000).toLocaleString('de-DE', { minimumFractionDigits: 1, maximumFractionDigits: 1 })} s`
}

const LANGS: Record<string, string> = {
  de: 'Deutsch',
  ger: 'Deutsch',
  deu: 'Deutsch',
  en: 'Englisch',
  eng: 'Englisch',
  fr: 'Französisch',
  fre: 'Französisch',
  fra: 'Französisch',
  es: 'Spanisch',
  spa: 'Spanisch',
  it: 'Italienisch',
  ita: 'Italienisch',
  ja: 'Japanisch',
  jpn: 'Japanisch',
  ko: 'Koreanisch',
  kor: 'Koreanisch',
  tr: 'Türkisch',
  tur: 'Türkisch',
  pt: 'Portugiesisch',
  por: 'Portugiesisch',
  ru: 'Russisch',
  rus: 'Russisch',
}

/** English language names as playlists spell them, mapped to the German word. */
const NAMES_DE: Record<string, string> = {
  german: 'Deutsch',
  english: 'Englisch',
  french: 'Französisch',
  spanish: 'Spanisch',
  italian: 'Italienisch',
  japanese: 'Japanisch',
  korean: 'Koreanisch',
  turkish: 'Türkisch',
  portuguese: 'Portugiesisch',
  russian: 'Russisch',
}
/** Playlist tags in track names, in plain German. */
const TAGS: [RegExp, string][] = [
  [/\s*[[(]\s*forced\s*[\])]/i, ' (nur Fremdsprache)'],
  [/\s*[[(]\s*(cc|sdh)\s*[\])]/i, ' (mit Geräuschen)'],
]

/** Readable German name for a playlist track: "Italian [Forced]" → "Italienisch (nur Fremdsprache)". */
export function trackLabel(track: { lang?: string | null; name?: string | null }, index: number, kind: string): string {
  const lang = track.lang ? LANGS[track.lang.toLowerCase()] : undefined
  let name = track.name?.trim() ?? ''
  if (lang && name.length <= 3) return lang
  for (const [en, de] of Object.entries(NAMES_DE)) name = name.replace(new RegExp(`^${en}\\b`, 'i'), de)
  for (const [re, de] of TAGS) name = name.replace(re, de)
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
