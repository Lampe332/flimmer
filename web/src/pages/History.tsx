import { useEffect, useRef, useState } from 'react'
import { Link } from 'react-router'
import Icon from '../components/Icon'
import Row from '../components/Row'
import { EmptyState, ErrorState, PageLoader } from '../components/States'
import { NewEpisodeCard, ProgressBar } from '../components/WatchCards'
import { titlePath, type HistoryEntry } from '../lib/api'
import { t, useI18n } from '../i18n'
import { locale, typeLabel } from '../lib/format'
import { epCode, remaining, useHistory, useHistoryDelete, useNewEpisodes } from '../lib/progress'
import { useTitle } from '../lib/title'

function dayLabel(d: Date, now: Date): string {
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime()
  const days = Math.round((start(now) - start(d)) / 86_400_000)
  if (days === 0) return t('hist.today')
  if (days === 1) return t('hist.yesterday')
  const year = d.getFullYear() === now.getFullYear() ? undefined : 'numeric'
  return new Intl.DateTimeFormat(locale(), { weekday: 'long', day: 'numeric', month: 'long', year }).format(d)
}

/**
 * One row per movie or episode: watching it again later (from a stream or a downloaded file) adds a
 * new server entry, the list keeps only the newest. `same` lists every entry a row stands for, so
 * removing the row removes all of them.
 */
function onePerTitle(all: HistoryEntry[]): { entries: HistoryEntry[]; same: Map<number, number[]> } {
  const first = new Map<string, number[]>()
  const entries: HistoryEntry[] = []
  for (const h of all) {
    const key = `${h.item.type}:${h.item.id}:${h.season ?? 0}:${h.episode ?? 0}`
    const ids = first.get(key)
    if (ids) ids.push(h.entry)
    else {
      first.set(key, [h.entry])
      entries.push(h)
    }
  }
  const same = new Map<number, number[]>()
  for (const ids of first.values()) if (ids[0] !== undefined) same.set(ids[0], ids)
  return { entries, same }
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
  const { t } = useI18n()
  useTitle(t('nav.history'))
  const history = useHistory()
  const fresh = useNewEpisodes()
  const remove = useHistoryDelete()
  const [confirm, setConfirm] = useState(false)

  if (history.error) return <ErrorState error={history.error} onRetry={() => void history.refetch()} />
  if (history.isPending) return <PageLoader />

  const { entries, same } = onePerTitle(history.data)
  const groups = byDay(entries)

  return (
    <div className="verlauf">
      <div className="page-head verlauf-head">
        <div>
          <h1 className="page-title">{t('nav.history')}</h1>
          {entries.length > 0 && <p className="page-sub">{t('hist.count', { n: entries.length })}</p>}
        </div>
        {entries.length > 0 && (
          <button type="button" className="btn btn-glass" onClick={() => setConfirm(true)}>
            <Icon name="trash" size={17} />
            {t('hist.clear')}
          </button>
        )}
      </div>

      {fresh.data && fresh.data.length > 0 && (
        <Row title={t('home.newEpisodes')}>
          {fresh.data.map((n) => (
            <NewEpisodeCard key={n.item.id} ep={n} />
          ))}
        </Row>
      )}

      {entries.length === 0 ? (
        <EmptyState
          icon="clock"
          title={t('hist.emptyTitle')}
          text={t('hist.emptyText')}
        >
          <Link to="/" className="btn btn-primary">
            {t('common.toHome')}
          </Link>
        </EmptyState>
      ) : (
        <div className="verlauf-days">
          {groups.map((g) => (
            <section key={g.key} className="hist-day" aria-label={g.label}>
              <h2 className="hist-day-title">{g.label}</h2>
              <ol className="hist-list">
                {g.entries.map((h) => (
                  <HistoryRow key={h.entry} h={h} onRemove={() => (same.get(h.entry) ?? [h.entry]).forEach((n) => remove.mutate(n))} />
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
  const { t } = useI18n()
  const code = epCode(h.season, h.episode)
  const tv = h.item.type === 'tv'
  const to = tv && h.season !== null ? `${titlePath(h.item)}?staffel=${h.season}` : titlePath(h.item)
  const time = new Intl.DateTimeFormat(locale(), { hour: '2-digit', minute: '2-digit' }).format(new Date(h.watched_at))
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
                {t('ep.seen')}
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
        aria-label={t('hist.removeLabel', { title: `${h.item.title}${code ? ` ${code}` : ''}` })}
        title={t('hist.removeTitle')}
        onClick={onRemove}
      >
        <Icon name="close" size={16} />
      </button>
    </li>
  )
}

function ConfirmClear({ count, onCancel, onConfirm }: { count: number; onCancel: () => void; onConfirm: () => void }) {
  const { t } = useI18n()
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
        <h2 id="clear-title">{t('hist.confirmTitle')}</h2>
        <p>{t('hist.confirmText', { n: count })}</p>
        <div className="actions">
          <button type="button" className="btn btn-glass" onClick={onCancel} autoFocus>
            {t('common.cancel')}
          </button>
          <button type="button" className="btn btn-danger" onClick={onConfirm}>
            <Icon name="trash" size={17} />
            {t('hist.clear')}
          </button>
        </div>
      </div>
    </dialog>
  )
}
