import { useState } from 'react'
import { Link } from 'react-router'
import { useI18n, type Key, type T } from '../../i18n'
import type { AudioLang, Filter, MinHeight, Scan, ScanResult, Settings, SortKey } from '../../lib/api'
import { formatBitrate, formatHeight, formatMs, langCode } from '../../lib/format'
import { fit, rank } from '../../lib/pick'
import { usePrefs } from '../../lib/prefs'
import type { TorrentFilter, TorrentMode } from '../../lib/prefs'
import type { Torrent, TorrentResult, TorrentSearch, Vpn } from '../../lib/torrents'
import Icon from '../Icon'
import { FilterBar, FilterEmpty } from '../TorrentFilters'
import { TorrentRow, VpnBadge, VpnNotice } from '../Torrents'

/** The torrent group under the provider list. */
type TorrentGroup = {
  results: TorrentResult[]
  /** The whole answer (facets, counts) for the filter bar. */
  data: TorrentSearch | undefined
  loading: boolean
  /** New filter counts are on the way. */
  busy: boolean
  filter: TorrentFilter
  mode: TorrentMode
  onFilter: (f: TorrentFilter) => void
  onMode: (m: TorrentMode) => void
  /** Torrents of the download list by infohash (their files are known). */
  known: ReadonlyMap<string, Torrent>
  error: unknown
  vpn: Vpn | undefined
  /** Infohash that plays right now. */
  active: string | null
  /** Infohash the automatic choice prefers over every provider. */
  best: string | null
  /** Infohashes already in the download list. */
  listed: ReadonlySet<string>
  onPlay: (r: TorrentResult) => void
  onDownload: (r: TorrentResult) => void
  downloading: string | null
  downloadError: unknown
}

type Props = {
  sniffer: boolean
  scan: Scan | undefined
  loading: boolean
  settings: Settings
  saveFailed: boolean
  onSettings: (next: Settings) => void
  /** Provider that is playing right now. */
  active: string | null
  /** True when the user picked the source by hand instead of letting Flimmer choose. */
  manual: boolean
  onPick: (providerId: string) => void
  onAuto: () => void
  onRescan: () => void
  torrents: TorrentGroup
}

const TORRENTS_SHOWN = 6

const heights = (t: T): { value: MinHeight; label: string }[] => [
  { value: 0, label: t('scan.any') },
  { value: 480, label: t('scan.from', { h: '480p' }) },
  { value: 720, label: t('scan.from', { h: '720p' }) },
  { value: 1080, label: t('scan.from', { h: '1080p' }) },
  { value: 2160, label: t('scan.only4k') },
]
const langs = (t: T): { value: AudioLang; label: string }[] => [
  { value: 'any', label: t('scan.any') },
  { value: 'de', label: t('lang.de') },
  { value: 'en', label: t('lang.en') },
]
const SORTS: { value: SortKey; label: Key; hint: Key }[] = [
  { value: 'quality', label: 'scan.sortQuality', hint: 'scan.sortQualityHint' },
  { value: 'bitrate', label: 'scan.sortBitrate', hint: 'scan.sortBitrateHint' },
  { value: 'speed', label: 'scan.sortSpeed', hint: 'scan.sortSpeedHint' },
]
const PREFETCH = [5, 10, 20, 30]

const ORDER: Record<ScanResult['status'], number> = { ok: 0, running: 1, pending: 2, no_stream: 3, error: 4 }

/** One line for the closed filter box, e.g. "ab 1080p · Deutsch · Schärfe". */
function summary(f: Filter, t: T): string {
  const h = heights(t).find((x) => x.value === f.min_height)
  const l = langs(t).find((x) => x.value === f.audio_lang)
  const s = SORTS.find((x) => x.value === f.sort)
  return [f.min_height ? h?.label : t('scan.anyQuality'), f.audio_lang === 'any' ? t('scan.anyLanguage') : l?.label, s && t(s.label)]
    .filter(Boolean)
    .join(' · ')
}

export default function ScanPanel(p: Props) {
  const { t } = useI18n()
  const HEIGHTS = heights(t)
  const LANGS = langs(t)
  const { filter } = p.settings
  const results = p.scan?.results ?? []
  const ranked = rank(results, filter)
  const best = ranked[0]?.provider_id ?? null
  const waiting = results.filter((r) => r.status === 'pending' || r.status === 'running')
  const failed = results.filter((r) => r.status === 'no_stream' || r.status === 'error').sort((a, z) => ORDER[a.status] - ORDER[z.status])
  // "Quellen ohne Stream ausblenden": no folded list of providers that came up empty.
  const { hideFailed } = usePrefs()
  const running = p.scan?.state === 'running'
  const setFilter = (patch: Partial<Filter>) => p.onSettings({ ...p.settings, filter: { ...filter, ...patch } })
  const minutes = PREFETCH.includes(p.settings.prefetch_minutes) ? PREFETCH : [...PREFETCH, p.settings.prefetch_minutes].sort((a, z) => a - z)
  const found = t('scan.found', { n: ranked.length })
  const sortHintKey = SORTS.find((s) => s.value === filter.sort)?.hint
  const sortHint = sortHintKey && t(sortHintKey)

  let state = t('scan.notYet')
  if (p.loading) state = t('common.loading')
  else if (running) state = t('scan.running', { found })
  else if (results.length > 0) state = t('scan.playable', { n: ranked.length })

  return (
    <aside className="scan" aria-label={t('scan.title')}>
      <header className="scan-head">
        <h2 className="scan-title">{t('scan.title')}</h2>
        <p className={`scan-state${running ? ' is-running' : ''}`} role="status">
          {running && <span className="bars" aria-hidden="true" />}
          {state}
        </p>
        {p.sniffer && (
          <button type="button" className="icon-btn" onClick={p.onRescan} disabled={running} aria-label={t('player.rescan')} title={t('player.rescan')}>
            <Icon name="refresh" size={17} />
          </button>
        )}
      </header>
      <p className="scan-intro">
        {p.sniffer ? t('scan.introApp') : t('scan.introBrowser')}
      </p>

      <details className="filters-box">
        <summary className="filters-summary">
          <Icon name="sliders" size={16} />
          <span className="filters-summary-text">
            <span className="filters-summary-label">{t('scan.auto')}</span>
            <span className="filters-summary-value">{summary(filter, t)}</span>
          </span>
          <Icon name="down" size={14} />
        </summary>
      <div className="filters" role="group" aria-label={t('scan.filterGroup')}>
        <label className="field">
          <span className="field-label">{t('scan.minQuality')}</span>
          <span className="select">
            <select value={filter.min_height} onChange={(e) => setFilter({ min_height: Number(e.target.value) as MinHeight })}>
              {HEIGHTS.map((h) => (
                <option key={h.value} value={h.value}>
                  {h.label}
                </option>
              ))}
            </select>
            <Icon name="down" size={14} />
          </span>
        </label>
        <label className="field">
          <span className="field-label">{t('scan.audio')}</span>
          <span className="select">
            <select value={filter.audio_lang} onChange={(e) => setFilter({ audio_lang: e.target.value as AudioLang })}>
              {LANGS.map((l) => (
                <option key={l.value} value={l.value}>
                  {l.label}
                </option>
              ))}
            </select>
            <Icon name="down" size={14} />
          </span>
        </label>
        <label className="field">
          <span className="field-label">{t('scan.prefetch')}</span>
          <span className="select">
            <select
              value={p.settings.prefetch_minutes}
              onChange={(e) => p.onSettings({ ...p.settings, prefetch_minutes: Number(e.target.value) })}
            >
              {minutes.map((m) => (
                <option key={m} value={m}>
                  {t('fmt.m', { m })}
                </option>
              ))}
            </select>
            <Icon name="down" size={14} />
          </span>
        </label>
        <div className="field field-wide">
          <span className="field-label" id="sort-label">
            {t('scan.tie')}
          </span>
          <div className="seg" role="radiogroup" aria-labelledby="sort-label">
            {SORTS.map((s) => (
              <button
                key={s.value}
                type="button"
                role="radio"
                aria-checked={filter.sort === s.value}
                className="seg-item"
                onClick={() => setFilter({ sort: s.value })}
              >
                {t(s.label)}
              </button>
            ))}
          </div>
          {sortHint && <p className="field-hint">{sortHint}</p>}
        </div>
        <p className="field-hint field-wide">{t('scan.filterNote')}</p>
        {p.saveFailed && (
          <p className="field-error" role="alert">
            {t('scan.saveFailed')}
          </p>
        )}
      </div>
      </details>

      {p.manual && best && (
        <button type="button" className="scan-auto" onClick={p.onAuto}>
          <Icon name="bolt" size={15} />
          {t('scan.backToAuto')}
        </button>
      )}

      {results.length === 0 ? (
        <p className="scan-empty">
          {p.loading ? t('scan.loadingList') : p.sniffer ? t('scan.startsSoon') : t('scan.noChecked')}
        </p>
      ) : (
        <ol className="scan-list">
          {ranked.map((r) => {
            const f = fit(r, filter)
            const isActive = r.provider_id === p.active
            const codes = [...new Set(r.audio.map((a) => langCode(a.lang)).filter((x): x is string => x !== null))]
            const details = [
              formatBitrate(r.max_bandwidth) && t('scan.bitrate', { v: formatBitrate(r.max_bandwidth) ?? '' }),
              formatMs(r.ms) && t('scan.answered', { v: formatMs(r.ms) ?? '' }),
              r.subtitles.length > 0 && t('scan.subsCount', { n: r.subtitles.length }),
            ]
              .filter(Boolean)
              .join(' · ')
            return (
              <li key={r.provider_id}>
                <button
                  type="button"
                  className={`source is-ok${isActive ? ' is-active' : ''}${f.height && f.lang ? '' : ' is-off'}`}
                  aria-pressed={isActive}
                  title={details || undefined}
                  onClick={() => p.onPick(r.provider_id)}
                >
                  <span className="source-main">
                    <span className="source-name">{r.name}</span>
                    {isActive ? (
                      <span className="tag tag-live">
                        <span className="bars" aria-hidden="true" />
                        {t('scan.live')}
                      </span>
                    ) : r.provider_id === best ? (
                      <span className="tag">{t('scan.best')}</span>
                    ) : !(f.height && f.lang) ? (
                      <span className="tag tag-dim">{t('scan.offFilter')}</span>
                    ) : null}
                  </span>
                  <span className="source-facts">
                    <span className={`fact fact-quality${f.height ? '' : ' is-miss'}`}>{formatHeight(r.max_height) ?? t('scan.unknownQuality')}</span>
                    {codes.length > 0 && <span className={`fact${f.lang ? '' : ' is-miss'}`}>{codes.slice(0, 4).join(' ')}</span>}
                    {r.subtitles.length > 0 && <span className="fact">{t('scan.subs')}</span>}
                  </span>
                </button>
              </li>
            )
          })}
          {ranked.length === 0 && running && <li className="scan-empty">{t('scan.noneYet')}</li>}
        </ol>
      )}

      {waiting.length > 0 && (
        <p className="scan-waiting">
          <span className="bars" aria-hidden="true" />
          {t('scan.waiting', { n: waiting.length })}
        </p>
      )}
      {failed.length > 0 && !hideFailed && (
        <details className="scan-failed">
          <summary>
            {t('scan.failed', { n: failed.length })}
            <Icon name="down" size={14} />
          </summary>
          <ul>
            {failed.map((r) => (
              <li key={r.provider_id} className={`source is-${r.status}`}>
                <span className="source-name">{r.name}</span>
                <span className="fact">{r.status === 'no_stream' ? t('scan.nothing') : (r.error ?? t('scan.error'))}</span>
              </li>
            ))}
          </ul>
        </details>
      )}

      <TorrentSection g={p.torrents} filter={filter} />
    </aside>
  )
}

function TorrentSection({ g, filter }: { g: TorrentGroup; filter: Filter }) {
  const { t } = useI18n()
  const [all, setAll] = useState(false)
  const shown = all ? g.results : g.results.slice(0, TORRENTS_SHOWN)
  const hidden = g.results.length - shown.length
  const anyListed = g.results.some((r) => g.listed.has(r.infohash))
  let state = t('tor.searching')
  if (!g.loading) state = g.error ? t('tor.searchFailedShort') : t('tor.count', { n: g.data?.matched ?? g.results.length })
  return (
    <section className="tgroup" aria-labelledby="tgroup-title">
      <header className="tgroup-head">
        <h3 className="tgroup-title" id="tgroup-title">
          {t('tor.group')}
        </h3>
        <p className={`scan-state${g.loading ? ' is-running' : ''}`}>
          {g.loading && <span className="bars" aria-hidden="true" />}
          {state}
        </p>
        <VpnBadge vpn={g.vpn} />
      </header>
      <VpnNotice vpn={g.vpn} compact />
      {!g.error && (g.data?.total ?? 0) > 0 && (
        <FilterBar data={g.data} busy={g.busy} filter={g.filter} mode={g.mode} onFilter={g.onFilter} onMode={g.onMode} compact />
      )}
      {g.loading ? null : g.error ? (
        <p className="scan-empty">{t('tor.searchFailed')}</p>
      ) : g.results.length === 0 ? (
        g.data && g.data.total > 0 ? (
          <FilterEmpty data={g.data} filter={g.filter} onFilter={g.onFilter} />
        ) : (
          <p className="scan-empty">{t('tor.none')}</p>
        )
      ) : (
        <ol className="scan-list tlist">
          {shown.map((r) => (
            <TorrentRow
              key={r.infohash}
              r={r}
              filter={filter}
              known={g.known.get(r.infohash)}
              active={r.infohash === g.active}
              best={r.infohash === g.best}
              inList={g.listed.has(r.infohash) || g.downloading === r.infohash}
              onPlay={() => g.onPlay(r)}
              onDownload={() => g.onDownload(r)}
            />
          ))}
        </ol>
      )}
      {hidden > 0 && (
        <button type="button" className="tgroup-more" onClick={() => setAll(true)}>
          {t('tor.more', { n: hidden })}
          <Icon name="down" size={14} />
        </button>
      )}
      {!!g.downloadError && (
        <p className="field-error" role="alert">
          {t('tor.downloadFailed')}
        </p>
      )}
      {anyListed && (
        <Link to="/downloads" className="tgroup-link">
          <Icon name="download" size={15} />
          {t('tor.toDownloads')}
        </Link>
      )}
    </section>
  )
}
