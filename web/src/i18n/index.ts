import { useCallback, useSyncExternalStore } from 'react'
import type { de } from './de'

/** Every UI text key. `de` is the source; every other dictionary is typed `Dict`, so keys always match. */
export type Key = keyof typeof de
export type Dict = Record<Key, string>
export type Vars = Record<string, string | number>
export type T = (key: Key, vars?: Vars) => string

/**
 * The UI languages. `locale` is the BCP 47 tag for Intl, `<html lang>` and the TMDB `language`
 * the server is asked for; `name` is the language's own name, so each one is findable from any other.
 * `load` fetches the dictionary on demand: only the language in use is downloaded.
 */
export const LANGUAGES = [
  { id: 'de', locale: 'de-DE', name: 'Deutsch', load: () => import('./de').then((m) => m.de) },
  { id: 'en', locale: 'en-US', name: 'English', load: () => import('./en').then((m) => m.en) },
  { id: 'fr', locale: 'fr-FR', name: 'Français', load: () => import('./fr').then((m) => m.fr) },
  { id: 'es', locale: 'es-ES', name: 'Español', load: () => import('./es').then((m) => m.es) },
  { id: 'it', locale: 'it-IT', name: 'Italiano', load: () => import('./it').then((m) => m.it) },
  { id: 'pt', locale: 'pt-BR', name: 'Português (Brasil)', load: () => import('./pt').then((m) => m.pt) },
  { id: 'ru', locale: 'ru-RU', name: 'Русский', load: () => import('./ru').then((m) => m.ru) },
  { id: 'pl', locale: 'pl-PL', name: 'Polski', load: () => import('./pl').then((m) => m.pl) },
  { id: 'tr', locale: 'tr-TR', name: 'Türkçe', load: () => import('./tr').then((m) => m.tr) },
  { id: 'ja', locale: 'ja-JP', name: '日本語', load: () => import('./ja').then((m) => m.ja) },
  { id: 'zh', locale: 'zh-CN', name: '简体中文', load: () => import('./zh').then((m) => m.zh) },
  { id: 'ko', locale: 'ko-KR', name: '한국어', load: () => import('./ko').then((m) => m.ko) },
] as const satisfies readonly { id: string; locale: string; name: string; load: () => Promise<Dict> }[]

export type Lang = (typeof LANGUAGES)[number]['id']
type Info = (typeof LANGUAGES)[number]

const STORE_KEY = 'flimmer.lang'
const BY_ID = new Map<string, Info>(LANGUAGES.map((l) => [l.id, l]))

function info(lang: Lang): Info {
  return BY_ID.get(lang) ?? LANGUAGES[0]
}

function isLang(v: string | null): v is Lang {
  return v !== null && BY_ID.has(v)
}

function stored(): Lang | null {
  try {
    const v = localStorage.getItem(STORE_KEY)
    return isLang(v) ? v : null
  } catch {
    return null
  }
}

/** Saved choice, else German (WebView2 often reports en-US even on German systems). */
function initial(): Lang {
  return stored() ?? 'de'
}

let current: Lang = initial()
const listeners = new Set<() => void>()
const dicts = new Map<Lang, Dict>()

function applyToDocument(lang: Lang) {
  document.documentElement.lang = info(lang).locale
  document.querySelector('meta[name="description"]')?.setAttribute('content', translate(lang, 'app.description'))
}

/** Loads the starting language; the app renders after this. */
export async function loadInitialLang(): Promise<void> {
  dicts.set(current, await info(current).load())
  applyToDocument(current)
}

export function getLang(): Lang {
  return current
}

/** The language a switch is waiting for (its dictionary is still loading). */
let wanted: Lang = current

export function setLang(lang: Lang): void {
  wanted = lang
  if (lang === current) return
  if (!dicts.has(lang)) {
    void info(lang)
      .load()
      .then((d) => {
        dicts.set(lang, d)
        if (wanted === lang) setLang(lang)
      })
      .catch(() => undefined)
    return
  }
  current = lang
  try {
    localStorage.setItem(STORE_KEY, lang)
  } catch {
    /* storage blocked: the choice just isn't remembered */
  }
  applyToDocument(lang)
  listeners.forEach((fn) => fn())
}

export function subscribeLang(fn: () => void): () => void {
  listeners.add(fn)
  return () => listeners.delete(fn)
}

/** BCP 47 locale for Intl and the TMDB `language` parameter, e.g. "pt-BR". */
export function localeOf(lang: Lang): string {
  return info(lang).locale
}

const pluralRules = new Map<string, Intl.PluralRules>()

/**
 * Plural texts separate their forms with "|": two forms are one|other, three are one|few|many
 * (Russian, Polish). The form is picked by `vars.n` with the language's own plural rules.
 */
function plural(lang: Lang, s: string, n: number): string {
  const parts = s.split('|')
  if (parts.length < 2) return s
  const loc = localeOf(lang)
  let rules = pluralRules.get(loc)
  if (!rules) {
    rules = new Intl.PluralRules(loc)
    pluralRules.set(loc, rules)
  }
  const cat = rules.select(n)
  const index = cat === 'one' ? 0 : parts.length === 2 ? 1 : cat === 'few' ? 1 : 2
  return parts[index] ?? s
}

/** Looks up `key` in the language and fills `{name}` placeholders. */
function translate(lang: Lang, key: Key, vars?: Vars): string {
  let s = dicts.get(lang)?.[key] ?? key
  if (!vars) return s
  if (typeof vars.n === 'number') s = plural(lang, s, vars.n)
  return s.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m))
}

/** Outside React (formatters, error texts): uses the current language at call time. */
export const t: T = (key, vars) => translate(current, key, vars)

/** Re-renders the component when the language changes. */
export function useI18n(): { lang: Lang; t: T; setLang: (lang: Lang) => void } {
  const lang = useSyncExternalStore(subscribeLang, getLang)
  const tr = useCallback<T>((key, vars) => translate(lang, key, vars), [lang])
  return { lang, t: tr, setLang }
}
