import { useEffect, useId, useState, type ReactNode } from 'react'
import { useI18n, type Key, type T } from '../i18n'
import { locale } from '../lib/format'
import { EMPTY_FILTER, FILTER_DIMS, setPrefs, usePrefs, type FilterDim, type TorrentFilter, type TorrentMode } from '../lib/prefs'
import { filterParams, type Facets, type TorrentSearch } from '../lib/torrents'
import Icon from './Icon'
import Sheet from './Sheet'
import { useIsPhone } from '../lib/platform'

/** The saved torrent filter and order (prefs, so it is the user's default everywhere). */
export function useTorrentFilter() {
  const prefs = usePrefs()
  const filter = prefs.torrentFilter
  const mode = prefs.torrentMode
  return {
    filter,
    mode,
    params: filterParams(filter, mode),
    setFilter: (next: TorrentFilter) => setPrefs({ torrentFilter: next }),
    setMode: (next: TorrentMode) => setPrefs({ torrentMode: next }),
  }
}

export const activeCount = (f: TorrentFilter) =>
  FILTER_DIMS.reduce((n, d) => n + (f.values[d]?.length ?? 0), 0) + Number(f.minSeeders > 0) + Number(f.minSizeGb > 0 || f.maxSizeGb > 0) + Number(f.cam)

/** Rows of the bar, in reading order; "Ton" holds both format and channels. */
const ROWS: { dims: FilterDim[]; label: Key }[] = [
  { dims: ['language'], label: 'tf.language' },
  { dims: ['sub'], label: 'tf.subs' },
  { dims: ['res'], label: 'tf.quality' },
  { dims: ['source'], label: 'tf.source' },
  { dims: ['codec'], label: 'tf.codec' },
  { dims: ['hdr'], label: 'tf.hdr' },
  { dims: ['audio', 'channels'], label: 'tf.audio' },
  { dims: ['group'], label: 'tf.group' },
]
const FIXED_ORDER: Partial<Record<FilterDim, string[]>> = {
  language: ['de', 'dl', 'multi', 'en', 'und'],
  res: ['2160', '1080', '720', 'sd', 'unknown'],
  source: ['Remux', 'BluRay', 'WEB-DL', 'WEBRip', 'HDTV', 'DVD'],
  channels: ['7.1', '5.1', '2.0', '1.0'],
}
const SEEDERS = [0, 1, 5, 20, 100]
/** Values shown before "more" for long lists (release groups, subtitle tags). */
const SHOWN = 8

let names: Intl.DisplayNames | null = null
let namesLocale = ''
function languageName(code: string): string {
  try {
    if (namesLocale !== locale()) {
      names = new Intl.DisplayNames([locale()], { type: 'language' })
      namesLocale = locale()
    }
    const n = names?.of(code)
    return n && n !== code ? n : code.toUpperCase()
  } catch {
    return code.toUpperCase()
  }
}

/** How a facet value reads on its chip. */
export function valueLabel(dim: FilterDim, value: string, t: T): string {
  if (value === 'unknown') return t('tf.unknown')
  // Subtitle facets are language codes too ("en", "multi"); a few are tags of their own ("VOSTFR").
  if (dim === 'language' || (dim === 'sub' && /^[a-z]{2,6}$/.test(value))) {
    if (value === 'und') return t('tf.langNone')
    if (value === 'multi') return 'Multi'
    if (value === 'dl') return t('tf.langDl')
    if (value === 'nordic') return t('tf.langNordic')
    return languageName(value)
  }
  if (dim === 'res') return value === '2160' ? '4K' : value === 'sd' ? 'SD' : `${value}p`
  if (dim === 'channels') return value === '1.0' ? t('tf.mono') : value === '2.0' ? t('tf.stereo') : value
  return value
}

function ordered(dim: FilterDim, counts: Record<string, number>, chosen: string[]): [string, number][] {
  const fixed = FIXED_ORDER[dim] ?? []
  const all = new Map(Object.entries(counts))
  for (const c of chosen) if (!all.has(c)) all.set(c, 0)
  return [...all.entries()]
    .filter(([v, n]) => n > 0 || chosen.includes(v))
    .sort(([a, x], [b, y]) => {
      // "Unknown" / "not stated" always comes last.
      const tail = (v: string) => Number(v === 'unknown' || v === 'und')
      if (tail(a) !== tail(b)) return tail(a) - tail(b)
      const ia = fixed.indexOf(a)
      const ib = fixed.indexOf(b)
      if (ia >= 0 || ib >= 0) return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib)
      return y - x || a.localeCompare(b)
    })
}

type BarProps = {
  data: TorrentSearch | undefined
  /** New counts are on the way (the list keeps the previous answer meanwhile). */
  busy: boolean
  filter: TorrentFilter
  mode: TorrentMode
  onFilter: (next: TorrentFilter) => void
  onMode: (next: TorrentMode) => void
  /** Player side panel: folded by default, one line until opened. */
  compact?: boolean
  /** Phone torrent search: rows always open, no own toggle, count or sheet (the page's filter sheet holds it). */
  bare?: boolean
}

/** Filter chips with live counts from the API, the order switch and "Zurücksetzen". */
export function FilterBar({ data, busy, filter, mode, onFilter, onMode, compact, bare }: BarProps) {
  const { t } = useI18n()
  // phones: the filters open as a bottom sheet, folded by default
  const phone = useIsPhone()
  const [open, setOpen] = useState(bare || (!compact && !phone))
  const id = useId()
  const facets: Facets = data?.facets ?? {}
  const n = activeCount(filter)
  const toggle = (dim: FilterDim, value: string) => {
    const list = filter.values[dim] ?? []
    const next = list.includes(value) ? list.filter((v) => v !== value) : [...list, value]
    onFilter({ ...filter, values: { ...filter.values, [dim]: next.length > 0 ? next : undefined } })
  }
  const camCount = facets.cam?.true ?? 0
  const summary = describe(filter, t)
  const rows = <FilterRows id={id} facets={facets} filter={filter} onFilter={onFilter} toggle={toggle} camCount={camCount} t={t} />

  return (
    <section className={`tf${compact ? ' is-compact' : ''}${busy ? ' is-busy' : ''}`} aria-label={t('tf.title')}>
      <div className="tf-head">
        <div className="seg tf-mode" role="radiogroup" aria-label={t('tf.mode')}>
          {(['fast', 'quality'] as const).map((m) => (
            <button key={m} type="button" role="radio" aria-checked={mode === m} className="seg-item" onClick={() => onMode(m)} title={t(m === 'fast' ? 'tf.fastHintSources' : 'tf.qualityHint')}>
              <Icon name={m === 'fast' ? 'bolt' : 'star'} size={14} />
              {t(m === 'fast' ? 'tf.fast' : 'tf.quality2')}
            </button>
          ))}
        </div>
        {!bare && (<>
        <button
          type="button"
          className={`tf-toggle${n > 0 ? ' has-active' : ''}`}
          aria-expanded={open}
          aria-controls={phone ? undefined : id}
          aria-haspopup={phone ? 'dialog' : undefined}
          onClick={() => setOpen((o) => !o)}
        >
          <Icon name="sliders" size={15} />
          <span className="tf-toggle-text">{n > 0 ? summary : t('tf.title')}</span>
          {n > 0 && <span className="tf-badge">{n}</span>}
          <Icon name="down" size={14} />
        </button>
        {data && (
          <p className="tf-count" role="status">
            {busy && <span className="bars" aria-hidden="true" />}
            {t('tf.matched', { a: data.matched.toLocaleString(locale()), b: data.total.toLocaleString(locale()) })}
          </p>
        )}
        <button type="button" className="tf-reset" onClick={() => onFilter(EMPTY_FILTER)} disabled={n === 0}>
          <Icon name="refresh" size={14} />
          {t('tf.reset')}
        </button>
        </>)}
      </div>
      <p className="field-hint tf-mode-hint">{t(mode === 'fast' ? 'tf.fastHintSources' : 'tf.qualityHint')}</p>

      {phone && !bare ? (
        <Sheet open={open} onClose={() => setOpen(false)} title={t('tf.title')} className="tf-sheet">
          {rows}
          <div className="sheet-actions">
            <button type="button" className="btn btn-glass" onClick={() => onFilter(EMPTY_FILTER)} disabled={n === 0}>
              {t('tf.reset')}
            </button>
            <button type="button" className="btn btn-primary" onClick={() => setOpen(false)}>
              {data ? t('tf.matched', { a: data.matched.toLocaleString(locale()), b: data.total.toLocaleString(locale()) }) : t('m.done')}
            </button>
          </div>
        </Sheet>
      ) : (
        open && rows
      )}
    </section>
  )
}

function FilterRows({ id, facets, filter, onFilter, toggle, camCount, t }: {
  id: string
  facets: Facets
  filter: TorrentFilter
  onFilter: (next: TorrentFilter) => void
  toggle: (dim: FilterDim, value: string) => void
  camCount: number
  t: T
}) {
  return (
        <div className="tf-rows" id={id}>
          {ROWS.map((row) => {
            const parts = row.dims.map((dim) => ({ dim, list: ordered(dim, facets[dim] ?? {}, filter.values[dim] ?? []) }))
            if (parts.every((p) => p.list.length === 0)) return null
            return (
              <FacetRow key={row.label} label={t(row.label)}>
                {parts.map(({ dim, list }) => (
                  <Chips key={dim} dim={dim} list={list} chosen={filter.values[dim] ?? []} onToggle={(v) => toggle(dim, v)} t={t} />
                ))}
                {row.dims[0] === 'source' && camCount + Number(filter.cam) > 0 && (
                  <button type="button" className="tf-chip is-warn" aria-pressed={filter.cam} onClick={() => onFilter({ ...filter, cam: !filter.cam })}>
                    {t('tf.cam')}
                    <span className="tf-n">{camCount}</span>
                  </button>
                )}
              </FacetRow>
            )
          })}
          <FacetRow label={t('tf.sources')}>
            {SEEDERS.map((s) => (
              <button key={s} type="button" className="tf-chip" aria-pressed={filter.minSeeders === s} onClick={() => onFilter({ ...filter, minSeeders: s })}>
                {s === 0 ? t('tf.all') : `${s}+`}
              </button>
            ))}
          </FacetRow>
          <FacetRow label={t('tf.size')}>
            <SizeRange filter={filter} onFilter={onFilter} />
          </FacetRow>
        </div>
  )
}

function FacetRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="tf-row" role="group" aria-label={label}>
      <span className="tf-label">{label}</span>
      <div className="tf-chips">{children}</div>
    </div>
  )
}

function Chips({ dim, list, chosen, onToggle, t }: { dim: FilterDim; list: [string, number][]; chosen: string[]; onToggle: (v: string) => void; t: T }) {
  const [all, setAll] = useState(false)
  const long = list.length > SHOWN + 2
  const shown = all || !long ? list : list.filter(([v], i) => i < SHOWN || chosen.includes(v))
  return (
    <>
      {shown.map(([v, n]) => (
        <button key={v} type="button" className={`tf-chip${n === 0 ? ' is-empty' : ''}`} aria-pressed={chosen.includes(v)} onClick={() => onToggle(v)}>
          {valueLabel(dim, v, t)}
          <span className="tf-n">{n.toLocaleString(locale())}</span>
        </button>
      ))}
      {long && (
        <button type="button" className="tf-more" onClick={() => setAll((a) => !a)} aria-expanded={all}>
          {all ? t('tf.less') : t('tf.moreN', { n: list.length - shown.length })}
        </button>
      )}
    </>
  )
}

/** "von … bis … GB"; applied when a field is left or Enter is pressed. */
function SizeRange({ filter, onFilter }: { filter: TorrentFilter; onFilter: (f: TorrentFilter) => void }) {
  const { t } = useI18n()
  const [min, setMin] = useState(filter.minSizeGb ? String(filter.minSizeGb) : '')
  const [max, setMax] = useState(filter.maxSizeGb ? String(filter.maxSizeGb) : '')
  useEffect(() => {
    setMin(filter.minSizeGb ? String(filter.minSizeGb) : '')
    setMax(filter.maxSizeGb ? String(filter.maxSizeGb) : '')
  }, [filter.minSizeGb, filter.maxSizeGb])
  const num = (s: string) => {
    const v = Number(s.replace(',', '.'))
    return Number.isFinite(v) && v > 0 ? Math.min(500, v) : 0
  }
  const apply = () => {
    const a = num(min)
    const b = num(max)
    if (a !== filter.minSizeGb || b !== filter.maxSizeGb) onFilter({ ...filter, minSizeGb: a, maxSizeGb: b })
  }
  const field = (value: string, set: (v: string) => void, label: string) => (
    <input
      className="input tf-size"
      inputMode="decimal"
      value={value}
      placeholder="0"
      aria-label={label}
      onChange={(e) => set(e.target.value)}
      onBlur={apply}
      onKeyDown={(e) => e.key === 'Enter' && apply()}
    />
  )
  return (
    <span className="tf-range">
      {field(min, setMin, t('tf.sizeMin'))}
      <span aria-hidden="true">–</span>
      {field(max, setMax, t('tf.sizeMax'))}
      <span className="tf-unit">GB</span>
    </span>
  )
}

/** One line for the folded bar: "Deutsch · 1080p · BluRay". */
function describe(f: TorrentFilter, t: T): string {
  const parts: string[] = []
  for (const dim of FILTER_DIMS) for (const v of f.values[dim] ?? []) parts.push(valueLabel(dim, v, t))
  if (f.minSeeders > 0) parts.push(t('tf.sourcesMin', { n: f.minSeeders }))
  if (f.minSizeGb > 0 || f.maxSizeGb > 0) parts.push(`${f.minSizeGb || 0}–${f.maxSizeGb || '∞'} GB`)
  if (f.cam) parts.push(t('tf.cam'))
  return parts.join(' · ')
}

/**
 * Nothing left: name the filters that removed everything. A chosen value whose own count is 0 (with the other
 * filters applied) is a culprit on its own; otherwise only the combination is.
 */
export function FilterEmpty({ data, filter, onFilter }: { data: TorrentSearch; filter: TorrentFilter; onFilter: (f: TorrentFilter) => void }) {
  const { t } = useI18n()
  // `back`: results without this filter. Facets count with all other filters applied, so for a dimension with one
  // value per torrent the sum is exact; where a torrent has several (languages, HDR) the largest count is a floor.
  const culprits: { label: string; drop: TorrentFilter; back?: number }[] = []
  for (const dim of FILTER_DIMS) {
    const chosen = filter.values[dim] ?? []
    if (chosen.length === 0) continue
    const label = chosen.map((v) => valueLabel(dim, v, t)).join(' / ')
    const counts = Object.values(data.facets[dim] ?? {})
    const multi = dim === 'language' || dim === 'sub' || dim === 'hdr'
    const back = multi ? Math.max(0, ...counts) : counts.reduce((a, b) => a + b, 0)
    culprits.push({ label, drop: { ...filter, values: { ...filter.values, [dim]: undefined } }, back })
  }
  if (filter.minSeeders > 0) culprits.push({ label: t('tf.sourcesMin', { n: filter.minSeeders }), drop: { ...filter, minSeeders: 0 } })
  if (filter.minSizeGb > 0 || filter.maxSizeGb > 0)
    culprits.push({ label: `${filter.minSizeGb || 0}–${filter.maxSizeGb || '∞'} GB`, drop: { ...filter, minSizeGb: 0, maxSizeGb: 0 } })
  // The filters whose removal alone brings results back are the ones to name.
  const alone = culprits.filter((c) => (c.back ?? 0) > 0)
  const text = alone.length > 0 ? t('tf.emptyBy', { what: alone.map((c) => c.label).join(', ') }) : t('tf.emptyCombo')
  return (
    <div className="tf-empty" role="status">
      <Icon name="sliders" size={20} />
      <div>
        <p className="tf-empty-title">{t('tf.emptyTitle', { n: data.total })}</p>
        <p>{text}</p>
        <div className="tf-empty-actions">
          {culprits.map((c) => (
            <button key={c.label} type="button" className="tf-chip" onClick={() => onFilter(c.drop)}>
              <Icon name="close" size={12} />
              {t('tf.without', { what: c.label })}
              {(c.back ?? 0) > 0 && <span className="tf-n">{c.back}</span>}
            </button>
          ))}
          <button type="button" className="tf-reset" onClick={() => onFilter(EMPTY_FILTER)}>
            <Icon name="refresh" size={14} />
            {t('tf.reset')}
          </button>
        </div>
      </div>
    </div>
  )
}
