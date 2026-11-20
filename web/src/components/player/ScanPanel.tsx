import type { AudioLang, Filter, MinHeight, Scan, ScanResult, Settings, SortKey } from '../../lib/api'
import { formatBitrate, formatHeight, formatMs, langCode } from '../../lib/format'
import { fit, rank } from '../../lib/pick'
import Icon from '../Icon'

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
}

const HEIGHTS: { value: MinHeight; label: string }[] = [
  { value: 0, label: 'Egal' },
  { value: 480, label: 'ab 480p' },
  { value: 720, label: 'ab 720p' },
  { value: 1080, label: 'ab 1080p' },
  { value: 2160, label: 'nur 4K' },
]
const LANGS: { value: AudioLang; label: string }[] = [
  { value: 'any', label: 'Egal' },
  { value: 'de', label: 'Deutsch' },
  { value: 'en', label: 'Englisch' },
]
const SORTS: { value: SortKey; label: string }[] = [
  { value: 'quality', label: 'Auflösung' },
  { value: 'bitrate', label: 'Bitrate' },
  { value: 'speed', label: 'Tempo' },
]
const PREFETCH = [5, 10, 20, 30]

const ORDER: Record<ScanResult['status'], number> = { ok: 0, running: 1, pending: 2, no_stream: 3, error: 4 }

export default function ScanPanel(p: Props) {
  const { filter } = p.settings
  const results = p.scan?.results ?? []
  const ranked = rank(results, filter)
  const best = ranked[0]?.provider_id ?? null
  const rest = results.filter((r) => !(r.status === 'ok' && r.stream_id)).sort((a, z) => ORDER[a.status] - ORDER[z.status])
  const checked = results.filter((r) => r.status !== 'pending' && r.status !== 'running').length
  const running = p.scan?.state === 'running'
  const setFilter = (patch: Partial<Filter>) => p.onSettings({ ...p.settings, filter: { ...filter, ...patch } })
  const minutes = PREFETCH.includes(p.settings.prefetch_minutes) ? PREFETCH : [...PREFETCH, p.settings.prefetch_minutes].sort((a, z) => a - z)

  let state = 'Noch nicht geprüft'
  if (p.loading) state = 'Lädt'
  else if (running) state = `${checked} von ${results.length} geprüft`
  else if (results.length > 0) state = ranked.length === 1 ? '1 Quelle spielbar' : `${ranked.length} Quellen spielbar`

  return (
    <aside className="scan" aria-label="Quellen">
      <header className="scan-head">
        <h2 className="scan-title">Quellen</h2>
        <p className={`scan-state${running ? ' is-running' : ''}`} role="status">
          {running && <span className="bars" aria-hidden="true" />}
          {state}
        </p>
        {p.sniffer && (
          <button type="button" className="icon-btn" onClick={p.onRescan} disabled={running} aria-label="Quellen neu prüfen" title="Quellen neu prüfen">
            <Icon name="refresh" size={17} />
          </button>
        )}
      </header>

      <div className="filters" role="group" aria-label="Filter für die automatische Auswahl">
        <label className="field">
          <span className="field-label">Qualität</span>
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
          <span className="field-label">Sprache</span>
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
          <span className="field-label">Vorladen</span>
          <span className="select">
            <select
              value={p.settings.prefetch_minutes}
              onChange={(e) => p.onSettings({ ...p.settings, prefetch_minutes: Number(e.target.value) })}
            >
              {minutes.map((m) => (
                <option key={m} value={m}>
                  {m} Min.
                </option>
              ))}
            </select>
            <Icon name="down" size={14} />
          </span>
        </label>
        <div className="field field-wide">
          <span className="field-label" id="sort-label">
            Bevorzugt
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
                {s.label}
              </button>
            ))}
          </div>
        </div>
        {p.saveFailed && (
          <p className="field-error" role="alert">
            Der Filter konnte nicht gespeichert werden und gilt nur für diese Sitzung.
          </p>
        )}
      </div>

      {p.manual && best && (
        <button type="button" className="scan-auto" onClick={p.onAuto}>
          <Icon name="bolt" size={15} />
          Wieder automatisch wählen
        </button>
      )}

      {results.length === 0 ? (
        <p className="scan-empty">
          {p.loading
            ? 'Quellen werden geladen.'
            : p.sniffer
              ? 'Noch keine Ergebnisse. Die Prüfung startet gleich.'
              : 'Für diesen Titel liegen keine geprüften Quellen vor.'}
        </p>
      ) : (
        <ol className="scan-list">
          {ranked.map((r) => {
            const f = fit(r, filter)
            const isActive = r.provider_id === p.active
            const langs = [...new Set(r.audio.map((a) => langCode(a.lang)).filter((x): x is string => x !== null))]
            return (
              <li key={r.provider_id}>
                <button
                  type="button"
                  className={`source is-ok${isActive ? ' is-active' : ''}${f.height && f.lang ? '' : ' is-off'}`}
                  aria-pressed={isActive}
                  onClick={() => p.onPick(r.provider_id)}
                >
                  <span className="source-main">
                    <span className="source-name">{r.name}</span>
                    {isActive ? (
                      <span className="tag tag-live">
                        <span className="bars" aria-hidden="true" />
                        Läuft
                      </span>
                    ) : r.provider_id === best ? (
                      <span className="tag">Beste Wahl</span>
                    ) : !(f.height && f.lang) ? (
                      <span className="tag tag-dim">außerhalb des Filters</span>
                    ) : null}
                  </span>
                  <span className="source-facts">
                    <span className={`fact fact-quality${f.height ? '' : ' is-miss'}`}>{formatHeight(r.max_height) ?? 'Auflösung unbekannt'}</span>
                    {formatBitrate(r.max_bandwidth) && <span className="fact">{formatBitrate(r.max_bandwidth)}</span>}
                    {langs.length > 0 && <span className={`fact${f.lang ? '' : ' is-miss'}`}>{langs.slice(0, 4).join(' ')}</span>}
                    {r.subtitles.length > 0 && <span className="fact">UT</span>}
                    {formatMs(r.ms) && <span className="fact fact-ms">{formatMs(r.ms)}</span>}
                  </span>
                </button>
              </li>
            )
          })}
          {rest.map((r) => (
            <li key={r.provider_id}>
              <div className={`source is-${r.status}`}>
                <span className="source-main">
                  <span className="source-name">{r.name}</span>
                  {r.status === 'running' && <span className="bars" aria-hidden="true" />}
                </span>
                <span className="source-facts">
                  <span className="fact">
                    {r.status === 'running'
                      ? 'wird geprüft'
                      : r.status === 'pending'
                        ? 'wartet'
                        : r.status === 'no_stream'
                          ? 'kein Stream gefunden'
                          : (r.error ?? 'Fehler bei der Prüfung')}
                  </span>
                </span>
              </div>
            </li>
          ))}
        </ol>
      )}
    </aside>
  )
}
