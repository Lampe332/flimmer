import { useEffect, useRef, useState } from 'react'
import { Link } from 'react-router'
import Icon from '../components/Icon'
import Row from '../components/Row'
import { EmptyState, ErrorState, PageLoader } from '../components/States'
import { NewEpisodeCard, ProgressBar } from '../components/WatchCards'
import { titlePath, type HistoryEntry } from '../lib/api'
import { typeLabel } from '../lib/format'
import { epCode, remaining, useHistory, useHistoryDelete, useNewEpisodes } from '../lib/progress'
import { useTitle } from '../lib/title'

const dayFmt = new Intl.DateTimeFormat('de-DE', { weekday: 'long', day: 'numeric', month: 'long' })
const dayYearFmt = new Intl.DateTimeFormat('de-DE', { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' })
const timeFmt = new Intl.DateTimeFormat('de-DE', { hour: '2-digit', minute: '2-digit' })

function dayLabel(d: Date, now: Date): string {
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime()
  const days = Math.round((start(now) - start(d)) / 86_400_000)
  if (days === 0) return 'Heute'
  if (days === 1) return 'Gestern'
  return (d.getFullYear() === now.getFullYear() ? dayFmt : dayYearFmt).format(d)
}

/** Entries grouped by local calendar day, newest day first (the server already sorts newest first). */
function byDay(entries: HistoryEntry[]): { label: string; key: string; entries: HistoryEntry[] }[] {
  const now = new Date()
  const groups: { label: string; key: string; entries: HistoryEntry[] }[] = []
  for (const h of entries) {
    const d = new Date(h.watched_at)
    const key = d.toDateString()
    const last = groups[groups.length - 1]
    if (last?.key === key) last.entries.push(h)
    else groups.push({ key, label: dayLabel(d, now), entries: [h] })
  }
  return groups
}

export default function History() {
  useTitle('Verlauf')
  const history = useHistory()
  const fresh = useNewEpisodes()
  const remove = useHistoryDelete()
  const [confirm, setConfirm] = useState(false)

  if (history.error) return <ErrorState error={history.error} onRetry={() => void history.refetch()} />
  if (history.isPending) return <PageLoader />

  const entries = history.data
  const groups = byDay(entries)

  return (
    <div className="verlauf">
      <div className="page-head verlauf-head">
        <div>
          <h1 className="page-title">Verlauf</h1>
          {entries.length > 0 && (
            <p className="page-sub">{entries.length === 1 ? '1 Eintrag' : `${entries.length} Einträge`}</p>
          )}
        </div>
        {entries.length > 0 && (
          <button type="button" className="btn btn-glass" onClick={() => setConfirm(true)}>
            <Icon name="trash" size={17} />
            Verlauf löschen
          </button>
        )}
      </div>

      {fresh.data && fresh.data.length > 0 && (
        <Row title="Neue Folgen deiner Serien">
          {fresh.data.map((n) => (
            <NewEpisodeCard key={n.item.id} ep={n} />
          ))}
        </Row>
      )}

      {entries.length === 0 ? (
        <EmptyState
          icon="clock"
          title="Noch nichts angesehen"
          text="Was du in Flimmer abspielst, landet hier, nach Tagen sortiert. Angefangene Titel findest du auf der Startseite unter Weiterschauen."
        >
          <Link to="/" className="btn btn-primary">
            Zur Startseite
          </Link>
        </EmptyState>
      ) : (
        <div className="verlauf-days">
          {groups.map((g) => (
            <section key={g.key} className="hist-day" aria-label={g.label}>
              <h2 className="hist-day-title">{g.label}</h2>
              <ol className="hist-list">
                {g.entries.map((h) => (
                  <HistoryRow key={h.entry} h={h} onRemove={() => remove.mutate(h.entry)} />
                ))}
              </ol>
            </section>
          ))}
        </div>
      )}

      {confirm && (
        <ConfirmClear
          count={entries.length}
          onCancel={() => setConfirm(false)}
          onConfirm={() => {
            remove.mutate('all')
            setConfirm(false)
          }}
        />
      )}
    </div>
  )
}

function HistoryRow({ h, onRemove }: { h: HistoryEntry; onRemove: () => void }) {
  const code = epCode(h.season, h.episode)
  const tv = h.item.type === 'tv'
  const to = tv && h.season !== null ? `${titlePath(h.item)}?staffel=${h.season}` : titlePath(h.item)
  const time = timeFmt.format(new Date(h.watched_at))
  return (
    <li className="hist">
      <time className="hist-time" dateTime={h.watched_at}>
        {time}
      </time>
      <Link to={to} className="hist-link">
        <div className="hist-thumb">
          {h.still || h.item.backdrop ? (
            <img src={h.still ?? h.item.backdrop ?? ''} alt="" loading="lazy" decoding="async" />
          ) : (
            <Icon name="film" size={22} />
          )}
          {!h.finished && h.duration > 0 && <ProgressBar position={h.position} duration={h.duration} />}
        </div>
        <div className="hist-text">
          <h3 className="hist-title">{h.item.title}</h3>
          <p className="hist-sub">
            {tv ? (
              <>
                <span className="card-ep">{code}</span>
                {h.episode_name && <span className="hist-ep-name">{h.episode_name}</span>}
              </>
            ) : (
              <span>
                {typeLabel(h.item.type)}
                {h.item.year ? `, ${h.item.year}` : ''}
              </span>
            )}
          </p>
          <p className="hist-meta">
            <span className="hist-time-inline">{time}</span>
            {h.finished ? (
              <span className="episode-state is-seen">
                <Icon name="check" size={13} stroke={2.4} />
                Gesehen
              </span>
            ) : (
              <span>{remaining(h.position, h.duration)}</span>
            )}
          </p>
        </div>
      </Link>
      <button
        type="button"
        className="icon-btn hist-remove"
        aria-label={`${h.item.title}${code ? ` ${code}` : ''} aus dem Verlauf entfernen`}
        title="Aus dem Verlauf entfernen"
        onClick={onRemove}
      >
        <Icon name="close" size={16} />
      </button>
    </li>
  )
}

function ConfirmClear({ count, onCancel, onConfirm }: { count: number; onCancel: () => void; onConfirm: () => void }) {
  const ref = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const d = ref.current
    if (!d) return
    d.showModal()
    return () => d.close()
  }, [])
  return (
    <dialog
      ref={ref}
      className="modal modal-confirm"
      aria-labelledby="clear-title"
      onClose={onCancel}
      onClick={(e) => {
        if (e.target === e.currentTarget) onCancel()
      }}
    >
      <div className="confirm">
        <h2 id="clear-title">Verlauf löschen?</h2>
        <p>
          {count === 1 ? 'Der Eintrag wird' : `Alle ${count} Einträge werden`} entfernt. Gespeicherte Positionen zum
          Weiterschauen und gesehene Folgen bleiben erhalten.
        </p>
        <div className="actions">
          <button type="button" className="btn btn-glass" onClick={onCancel} autoFocus>
            Abbrechen
          </button>
          <button type="button" className="btn btn-danger" onClick={onConfirm}>
            <Icon name="trash" size={17} />
            Verlauf löschen
          </button>
        </div>
      </div>
    </dialog>
  )
}
