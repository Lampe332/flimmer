import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router'
import { useI18n } from '../i18n'
import { codeText } from '../lib/errors'
import { refreshTorrents, torrentApi, torrentPlayPath, type TorrentEvent } from '../lib/torrents'
import Icon from './Icon'

const SHOW_MS = 7000
const UNDO_MS = 5000

type Undo = { id: number; text: string; undo: () => void }
let undoNow: Undo | null = null
const undoSubs = new Set<() => void>()
const setUndo = (u: Undo | null) => {
  undoNow = u
  undoSubs.forEach((f) => f())
}
/** "Entfernt: Dune" with "Rückgängig" for 5 s; a newer one replaces it. */
export function showUndo(text: string, undo: () => void) {
  setUndo({ id: Date.now(), text, undo })
}
const useUndo = () =>
  useSyncExternalStore(
    (on) => {
      undoSubs.add(on)
      return () => undoSubs.delete(on)
    },
    () => undoNow,
  )

/**
 * Torrent events as toasts: "Fertig: …", "Fehlgeschlagen: …", "Automatisch geladen: …". Only events that happen
 * while Flimmer is open; the first answer only sets where to start.
 */
export default function Toasts() {
  const { t } = useI18n()
  const qc = useQueryClient()
  const since = useRef<number | null>(null)
  const [shown, setShown] = useState<TorrentEvent[]>([])
  const undo = useUndo()
  useEffect(() => {
    if (!undo) return
    const timer = window.setTimeout(() => {
      if (undoNow?.id === undo.id) setUndo(null)
    }, UNDO_MS)
    return () => window.clearTimeout(timer)
  }, [undo])
  const events = useQuery({
    queryKey: ['torrent-events'],
    queryFn: () => torrentApi.events(since.current ?? 0),
    refetchInterval: 4000,
    retry: false,
    gcTime: 0,
  })
  useEffect(() => {
    const d = events.data
    if (!d) return
    const first = since.current === null
    since.current = d.last
    if (first || d.events.length === 0) return
    refreshTorrents(qc)
    setShown((old) => [...old, ...d.events].slice(-4))
  }, [events.data, qc])
  useEffect(() => {
    if (shown.length === 0) return
    const timer = window.setTimeout(() => setShown((s) => s.slice(1)), SHOW_MS)
    return () => window.clearTimeout(timer)
  }, [shown])

  const line = (e: TorrentEvent) => t(e.kind === 'finished' ? 'toast.finished' : e.kind === 'failed' ? 'toast.failed' : 'toast.grabbed', { name: label(e) })
  if (shown.length === 0 && !undo) return null
  return (
    <div className="toasts" role="status" aria-live="polite">
      {undo && (
        <div key={`undo-${undo.id}`} className="toast is-undo">
          <Icon name="trash" size={18} />
          <div className="toast-body">
            <p className="toast-title">{undo.text}</p>
            <button
              type="button"
              className="toast-link"
              onClick={() => {
                setUndo(null)
                undo.undo()
              }}
            >
              {t('wl.undo')}
            </button>
          </div>
          <button type="button" className="icon-btn toast-close" onClick={() => setUndo(null)} aria-label={t('toast.close')}>
            <Icon name="close" size={15} />
          </button>
        </div>
      )}
      {shown.map((e) => (
        <div key={e.id} className={`toast is-${e.kind}`}>
          <Icon name={e.kind === 'finished' ? 'check' : e.kind === 'failed' ? 'warn' : 'download'} size={18} />
          <div className="toast-body">
            <p className="toast-title">{line(e)}</p>
            {e.kind === 'failed' && <p className="toast-text">{codeText(e.code)}</p>}
            {e.kind === 'finished' && (
              <Link to={torrentPlayPath(e.torrent_id)} className="toast-link">
                {t('common.play')}
              </Link>
            )}
            {e.kind !== 'finished' && (
              <Link to="/downloads" className="toast-link">
                {t('tor.toDownloads')}
              </Link>
            )}
          </div>
          <button type="button" className="icon-btn toast-close" onClick={() => setShown((s) => s.filter((x) => x.id !== e.id))} aria-label={t('toast.close')}>
            <Icon name="close" size={15} />
          </button>
        </div>
      ))}
    </div>
  )
}

/** "Fight Club (1999)", "Dark S01E03", else the torrent name. */
function label(e: TorrentEvent): string {
  const m = e.media
  if (!m) return e.name
  if (m.type === 'tv' && m.season && m.episode) return `${m.title} S${String(m.season).padStart(2, '0')}E${String(m.episode).padStart(2, '0')}`
  return m.year ? `${m.title} (${m.year})` : m.title
}
