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

/** ISO 639-2 codes (files and playlists use them) of languages Intl names by their two-letter code. */
const THREE: Record<string, string> = {
  ger: 'de', deu: 'de', eng: 'en', fre: 'fr', fra: 'fr', spa: 'es', ita: 'it', jpn: 'ja', kor: 'ko', tur: 'tr',
  por: 'pt', rus: 'ru', pol: 'pl', chi: 'zh', zho: 'zh', ara: 'ar', dut: 'nl', nld: 'nl', swe: 'sv', nor: 'no',
  nob: 'nb', dan: 'da', fin: 'fi', hin: 'hi', hun: 'hu', cze: 'cs', ces: 'cs', gre: 'el', ell: 'el', heb: 'he',
  tha: 'th', vie: 'vi', ukr: 'uk', rum: 'ro', ron: 'ro', ind: 'id', may: 'ms', msa: 'ms', bul: 'bg', hrv: 'hr',
  srp: 'sr', slo: 'sk', slk: 'sk', slv: 'sl', est: 'et', lav: 'lv', lit: 'lt', per: 'fa', fas: 'fa', cat: 'ca',
  ice: 'is', isl: 'is', tam: 'ta', tel: 'te', ben: 'bn', urd: 'ur',
}
/** Words in English track names that stand for a language with a region. */
const ALIASES: Record<string, string> = { brazilian: 'pt-BR', castilian: 'es-ES', latino: 'es-419', flemish: 'nl-BE', mandarin: 'zh', cantonese: 'yue' }
/** Playlist tags in track names, in plain words. */
const TAGS: [RegExp, Key][] = [
  [/\s*[[(]?\s*\bforced\b\s*[\])]?/i, 'track.forced'],
  [/\s*[[(]?\s*\b(?:cc|sdh)\b\s*[\])]?/i, 'track.cc'],
]

/** "ger" → "de", "es_419" → "es-419"; null for "und" and other non-languages. */
function langTag(lang: string): string | null {
  const [head = '', ...rest] = lang.trim().replace(/_/g, '-').split('-')
  const base = THREE[head.toLowerCase()] ?? head.toLowerCase()
  if (!/^[a-z]{2,3}$/.test(base) || ['und', 'mul', 'zxx', 'mis'].includes(base)) return null
  return [base, ...rest].join('-')
}

const namers = new Map<string, Intl.DisplayNames>()
function languageName(tag: string, loc: string): string | null {
  try {
    let n = namers.get(loc)
    if (!n) namers.set(loc, (n = new Intl.DisplayNames([loc], { type: 'language', fallback: 'none' })))
    const name = n.of(tag)
    return name ? name.charAt(0).toLocaleUpperCase(loc) + name.slice(1) : null
  } catch {
    return null
  }
}

/** Language names (English, the language's own, the UI's) → tag, for tracks that only bring a name. */
const byName = new Map<string, Map<string, string>>()
function nameIndex(loc: string): Map<string, string> {
  let m = byName.get(loc)
  if (m) return m
  m = new Map(Object.entries(ALIASES))
  for (const code of new Set(Object.values(THREE))) {
    for (const l of ['en', code, loc]) {
      const name = languageName(code, l)?.toLowerCase()
      if (name && !m.has(name)) m.set(name, code)
    }
  }
  byName.set(loc, m)
  return m
}

/** A language name at the start of `name` ("Spanish (Latin America)", "Deutsch [Forced]"): its tag and length. */
function leadingLanguage(name: string, loc: string): { tag: string; len: number } | null {
  const word = /^[\p{L}]+/u.exec(name)?.[0]
  const tag = word && nameIndex(loc).get(word.toLowerCase())
  return word && tag ? { tag, len: word.length } : null
}

/**
 * Name of an audio / subtitle track in the UI language, the same way for every source: "ger", "German",
 * "Deutsch" all read "Deutsch" in a German UI. Tags become plain words ("Italian [Forced]" → "Italienisch (nur
 * Fremdsprache)"); other words of the name stay ("English Commentary" → "Englisch Commentary").
 */
export function trackLabel(track: { lang?: string | null; name?: string | null }, index: number, kind: string): string {
  const loc = locale()
  const name = track.name?.trim() ?? ''
  let tag = track.lang ? langTag(track.lang) : null
  let rest = name
  const lead = leadingLanguage(name, loc)
  if (lead) {
    tag ??= lead.tag
    rest = name.slice(lead.len)
    // "Brazilian Portuguese": the second language word adds nothing.
    const again = leadingLanguage(rest.trimStart(), loc)
    if (again) rest = rest.trimStart().slice(again.len)
  } else if (tag && name.length <= 3) {
    rest = ''
  }
  const lang = tag ? languageName(tag, loc) : null
  let tags = ''
  for (const [re, key] of TAGS) {
    if (re.test(rest)) {
      rest = rest.replace(re, '')
      tags += t(key)
    }
  }
  rest = rest.replace(/^[\s\-–—:|·,]+|[\s\-–—:|·,]+$/g, '')
  // "(Latin America)" next to a name that already says the region.
  if (lang?.includes('(') && /^\(.*\)$/.test(rest)) rest = ''
  const base = lang ? [lang, rest].filter(Boolean).join(' ') : rest || `${kind} ${index + 1}`
  return base + tags
}

/** Labels made unique: a second "Spanisch" becomes "Spanisch 2". */
export function uniqueLabels(labels: string[]): string[] {
  const seen = new Map<string, number>()
  return labels.map((l) => {
    const n = (seen.get(l) ?? 0) + 1
    seen.set(l, n)
    return n === 1 ? l : `${l} ${n}`
  })
}

/** Short language code for badges: DE, EN, or the raw code. */
export function langCode(lang: string | null): string | null {
  if (!lang || lang === 'und') return null
  const l = lang.toLowerCase()
  if (['de', 'ger', 'deu'].includes(l)) return 'DE'
  if (['en', 'eng'].includes(l)) return 'EN'
  return l.slice(0, 3).toUpperCase()
}
