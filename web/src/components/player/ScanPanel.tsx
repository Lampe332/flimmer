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
const SORTS: { value: SortKey; label: string; hint: string }[] = [
  { value: 'quality', label: 'Schärfe', hint: 'Die höchste Auflösung gewinnt.' },
  { value: 'bitrate', label: 'Datenrate', hint: 'Die meisten Bilddaten pro Sekunde gewinnen, oft das ruhigste Bild.' },
  { value: 'speed', label: 'Schneller Start', hint: 'Die Quelle, die am schnellsten geantwortet hat, gewinnt.' },
]
const PREFETCH = [5, 10, 20, 30]

const ORDER: Record<ScanResult['status'], number> = { ok: 0, running: 1, pending: 2, no_stream: 3, error: 4 }

/** One line for the closed filter box, e.g. "ab 1080p · Deutsch · Schärfe". */
function summary(f: Filter): string {
  const h = HEIGHTS.find((x) => x.value === f.min_height)
  const l = LANGS.find((x) => x.value === f.audio_lang)
  const s = SORTS.find((x) => x.value === f.sort)
  return [f.min_height ? h?.label : 'jede Qualität', f.audio_lang === 'any' ? 'jede Sprache' : l?.label, s?.label]
    .filter(Boolean)
    .join(' · ')
}

export default function ScanPanel(p: Props) {
  const { filter } = p.settings
  const results = p.scan?.results ?? []
  const ranked = rank(results, filter)
  const best = ranked[0]?.provider_id ?? null
  const waiting = results.filter((r) => r.status === 'pending' || r.status === 'running')
  const failed = results.filter((r) => r.status === 'no_stream' || r.status === 'error').sort((a, z) => ORDER[a.status] - ORDER[z.status])
  const running = p.scan?.state === 'running'
  const setFilter = (patch: Partial<Filter>) => p.onSettings({ ...p.settings, filter: { ...filter, ...patch } })
  const minutes = PREFETCH.includes(p.settings.prefetch_minutes) ? PREFETCH : [...PREFETCH, p.settings.prefetch_minutes].sort((a, z) => a - z)
  const found = ranked.length === 1 ? '1 gefunden' : `${ranked.length} gefunden`
  const sortHint = SORTS.find((s) => s.value === filter.sort)?.hint

  let state = 'Noch nicht gesucht'
  if (p.loading) state = 'Lädt'
  else if (running) state = `Suche läuft · ${found}`
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
          <button type="button" className="icon-btn" onClick={p.onRescan} disabled={running} aria-label="Quellen neu suchen" title="Quellen neu suchen">
            <Icon name="refresh" size={17} />
          </button>
        )}
      </header>
      <p className="scan-intro">
        {p.sniffer
          ? 'Flimmer fragt alle Anbieter und spielt die beste Quelle von selbst. Klick eine andere an, wenn es ruckelt.'
          : 'Hier stehen die Quellen, die die Flimmer-App schon gefunden hat. Klick eine an, um sie zu spielen.'}
      </p>

      <details className="filters-box">
        <summary className="filters-summary">
          <Icon name="sliders" size={16} />
          <span className="filters-summary-text">
            <span className="filters-summary-label">Automatische Auswahl</span>
            <span className="filters-summary-value">{summary(filter)}</span>
          </span>
          <Icon name="down" size={14} />
        </summary>
      <div className="filters" role="group" aria-label="Filter für die automatische Auswahl">
        <label className="field">
          <span className="field-label">Mindestqualität</span>
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
          <span className="field-label">Tonspur</span>
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
            Bei Gleichstand zählt
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
          {sortHint && <p className="field-hint">{sortHint}</p>}
        </div>
        <p className="field-hint field-wide">
          Quellen, die nicht passen, bleiben in der Liste und lassen sich weiter anklicken. Vorladen: so viele Minuten legt
          Flimmer vorab auf die Festplatte, damit nichts ruckelt.
        </p>
        {p.saveFailed && (
          <p className="field-error" role="alert">
            Der Filter konnte nicht gespeichert werden und gilt nur für diese Sitzung.
          </p>
        )}
      </div>
      </details>

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
            const details = [
              formatBitrate(r.max_bandwidth) && `Datenrate ${formatBitrate(r.max_bandwidth)}`,
              formatMs(r.ms) && `geantwortet nach ${formatMs(r.ms)}`,
              r.subtitles.length > 0 && `${r.subtitles.length} Untertitel`,
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
                        Läuft
                      </span>
                    ) : r.provider_id === best ? (
                      <span className="tag">Beste Wahl</span>
                    ) : !(f.height && f.lang) ? (
                      <span className="tag tag-dim">passt nicht zum Filter</span>
                    ) : null}
                  </span>
                  <span className="source-facts">
                    <span className={`fact fact-quality${f.height ? '' : ' is-miss'}`}>{formatHeight(r.max_height) ?? 'Qualität unbekannt'}</span>
                    {langs.length > 0 && <span className={`fact${f.lang ? '' : ' is-miss'}`}>{langs.slice(0, 4).join(' ')}</span>}
                    {r.subtitles.length > 0 && <span className="fact">Untertitel</span>}
                  </span>
                </button>
              </li>
            )
          })}
          {ranked.length === 0 && running && <li className="scan-empty">Noch keine Quelle gefunden.</li>}
        </ol>
      )}

      {waiting.length > 0 && (
        <p className="scan-waiting">
          <span className="bars" aria-hidden="true" />
          {waiting.length === 1 ? 'Ein Anbieter wird noch gefragt' : `${waiting.length} Anbieter werden noch gefragt`}
        </p>
      )}
      {failed.length > 0 && (
        <details className="scan-failed">
          <summary>
            {failed.length === 1 ? '1 Anbieter ohne Stream' : `${failed.length} Anbieter ohne Stream`}
            <Icon name="down" size={14} />
          </summary>
          <ul>
            {failed.map((r) => (
              <li key={r.provider_id} className={`source is-${r.status}`}>
                <span className="source-name">{r.name}</span>
                <span className="fact">{r.status === 'no_stream' ? 'nichts gefunden' : (r.error ?? 'Fehler')}</span>
              </li>
            ))}
          </ul>
        </details>
      )}
    </aside>
  )
}
