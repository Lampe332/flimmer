import { useSyncExternalStore } from 'react'
import { LANGUAGES, getLang, setLang, subscribeLang, type Lang } from '../i18n'
import { request } from './api'

/**
 * UI preferences. The server keeps them (`/api/prefs`, part of export/import and safe across a port change);
 * localStorage holds a copy so the first paint already has the right accent and motion.
 */
export type Accent = 'amber' | 'rose' | 'mint' | 'sky'
export type StartPage = 'home' | 'continue' | 'watchlist'
/** "off" / "auto" | "ui" (the app language) | a language id. */
export type TrackLang = string

export interface Prefs {
  lang: Lang | null
  accent: Accent
  reduceMotion: boolean
  start: StartPage
  autoNext: boolean
  autoNextSeconds: number
  subLang: TrackLang
  audioLang: TrackLang
  /** 0..1, used when "remember volume" is off (and as the first volume). */
  volume: number
  rememberVolume: boolean
  autoFullscreen: boolean
  hideFailed: boolean
  tourDone: boolean
}

export const ACCENTS: Accent[] = ['amber', 'rose', 'mint', 'sky']
export const START_PAGES: { id: StartPage; path: string }[] = [
  { id: 'home', path: '/' },
  { id: 'continue', path: '/verlauf' },
  { id: 'watchlist', path: '/merkliste' },
]
export const COUNTDOWNS = [5, 8, 10, 15, 20, 30]

export const DEFAULT_PREFS: Prefs = {
  lang: null,
  accent: 'amber',
  reduceMotion: false,
  start: 'home',
  autoNext: true,
  autoNextSeconds: 8,
  subLang: 'off',
  audioLang: 'auto',
  volume: 1,
  rememberVolume: true,
  autoFullscreen: false,
  hideFailed: false,
  tourDone: false,
}

const STORE_KEY = 'flimmer.prefs'
const LANG_IDS = new Set<string>(LANGUAGES.map((l) => l.id))

const bool = (v: unknown, d: boolean) => (typeof v === 'boolean' ? v : d)
const oneOf = <T extends string>(v: unknown, list: readonly T[], d: T): T => (list.includes(v as T) ? (v as T) : d)
const trackLang = (v: unknown, base: string, d: string) =>
  typeof v === 'string' && (v === base || v === 'ui' || LANG_IDS.has(v)) ? v : d

/** Unknown or broken values fall back to the default, field by field. */
export function cleanPrefs(raw: unknown): Prefs {
  const r = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>
  const d = DEFAULT_PREFS
  const num = (v: unknown, min: number, max: number, def: number) =>
    typeof v === 'number' && Number.isFinite(v) ? Math.min(max, Math.max(min, v)) : def
  return {
    lang: typeof r.lang === 'string' && LANG_IDS.has(r.lang) ? (r.lang as Lang) : null,
    accent: oneOf(r.accent, ACCENTS, d.accent),
    reduceMotion: bool(r.reduceMotion, d.reduceMotion),
    start: oneOf(r.start, START_PAGES.map((p) => p.id), d.start),
    autoNext: bool(r.autoNext, d.autoNext),
    autoNextSeconds: Math.round(num(r.autoNextSeconds, 3, 30, d.autoNextSeconds)),
    subLang: trackLang(r.subLang, 'off', d.subLang),
    audioLang: trackLang(r.audioLang, 'auto', d.audioLang),
    volume: num(r.volume, 0, 1, d.volume),
    rememberVolume: bool(r.rememberVolume, d.rememberVolume),
    autoFullscreen: bool(r.autoFullscreen, d.autoFullscreen),
    hideFailed: bool(r.hideFailed, d.hideFailed),
    tourDone: bool(r.tourDone, d.tourDone),
  }
}

function readLocal(): Prefs {
  try {
    const raw = localStorage.getItem(STORE_KEY)
    return cleanPrefs(raw ? JSON.parse(raw) : {})
  } catch {
    return DEFAULT_PREFS
  }
}

function writeLocal(p: Prefs) {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify(p))
  } catch {
    /* storage blocked: the server copy still has them */
  }
}

let current: Prefs = readLocal()
let loaded = false
const listeners = new Set<() => void>()

/** Accent and motion live on <html>, so plain CSS follows them. */
function apply(p: Prefs) {
  const html = document.documentElement
  html.dataset.accent = p.accent
  if (p.reduceMotion) html.dataset.motion = 'reduce'
  else delete html.dataset.motion
}
apply(current)

function emit() {
  listeners.forEach((fn) => fn())
}

export function getPrefs(): Prefs {
  return current
}

/** Server copy has arrived (the start page waits for it). */
export function prefsLoaded(): boolean {
  return loaded
}

let saveTimer: number | undefined
function saveSoon() {
  window.clearTimeout(saveTimer)
  saveTimer = window.setTimeout(() => {
    void request<void>('/prefs', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(current) }).catch(
      () => undefined,
    )
  }, 250)
}

export function setPrefs(patch: Partial<Prefs>): void {
  current = cleanPrefs({ ...current, ...patch })
  writeLocal(current)
  apply(current)
  emit()
  saveSoon()
}

/** Replaces everything with the server's copy (start, import, reset). */
export async function loadPrefs(): Promise<Prefs> {
  try {
    const remote = await request<unknown>('/prefs')
    const empty = !remote || (typeof remote === 'object' && Object.keys(remote).length === 0)
    // Nothing on the server yet (first start after an update): keep what this browser had and store it.
    current = empty ? current : cleanPrefs(remote)
    if (empty) saveSoon()
  } catch {
    /* server unreachable: the local copy stays */
  }
  loaded = true
  writeLocal(current)
  apply(current)
  if (current.lang && current.lang !== getLang()) setLang(current.lang)
  emit()
  return current
}

/** Forget the local copy (after a reset), back to defaults. */
export function resetLocalPrefs(): void {
  current = DEFAULT_PREFS
  writeLocal(current)
  apply(current)
  emit()
}

// A language switch is a preference too.
subscribeLang(() => {
  if (current.lang !== getLang()) setPrefs({ lang: getLang() })
})

function subscribe(fn: () => void) {
  listeners.add(fn)
  return () => listeners.delete(fn)
}

export function usePrefs(): Prefs {
  return useSyncExternalStore(subscribe, getPrefs)
}

/** Track language from hls.js ("de", "deu", "ger", "en-US" …) against a preference ("ui" or a language id). */
export function matchesLang(trackLang: string | null, want: TrackLang): boolean {
  if (!trackLang) return false
  const id = want === 'ui' ? getLang() : want
  const l = trackLang.toLowerCase().split(/[-_]/)[0] ?? ''
  const three: Record<string, string[]> = {
    de: ['deu', 'ger'],
    en: ['eng'],
    fr: ['fra', 'fre'],
    es: ['spa'],
    it: ['ita'],
    pt: ['por'],
    ru: ['rus'],
    pl: ['pol'],
    tr: ['tur'],
    ja: ['jpn'],
    zh: ['zho', 'chi'],
    ko: ['kor'],
  }
  return l === id || (three[id] ?? []).includes(l)
}
