import type { ReactNode } from 'react'
import { Link, useLocation } from 'react-router'
import { ApiError } from '../lib/api'
import Icon from './Icon'

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const { pathname } = useLocation()
  const status = error instanceof ApiError ? error.status : -1
  let title = 'Das hat nicht geklappt'
  let text = error instanceof Error ? error.message : 'Unbekannter Fehler.'
  if (status === 0) {
    title = 'Server nicht erreichbar'
    text = 'Flimmer bekommt keine Antwort vom lokalen Server. Starte flimmer.exe und versuche es erneut.'
  } else if (status === 404) {
    title = 'Nicht gefunden'
    text = 'Diesen Titel gibt es bei TMDB nicht (mehr).'
  } else if (status >= 500) {
    title = 'TMDB antwortet gerade nicht'
    text = `${text}. Prüfe die Internetverbindung und den API-Schlüssel in config.json.`
  }
  return (
    <div className="state" role="alert">
      <h2>{title}</h2>
      <p>{text}</p>
      <div className="state-actions">
        {onRetry && status !== 404 && (
          <button type="button" className="btn btn-primary" onClick={onRetry}>
            <Icon name="refresh" size={18} />
            Erneut versuchen
          </button>
        )}
        {pathname !== '/' && (
          <Link to="/" className="btn btn-ghost">
            Zur Startseite
          </Link>
        )}
      </div>
    </div>
  )
}

export function EmptyState({ title, text, children }: { title: string; text: string; children?: ReactNode }) {
  return (
    <div className="state">
      <h2>{title}</h2>
      <p>{text}</p>
      {children && <div className="state-actions">{children}</div>}
    </div>
  )
}

export function CardSkeletons({ count = 8 }: { count?: number }) {
  return (
    <>
      {Array.from({ length: count }, (_, i) => (
        <div key={i} className="card card-skel" aria-hidden="true">
          <div className="card-poster skel" />
          <div className="skel skel-line" />
          <div className="skel skel-line short" />
        </div>
      ))}
    </>
  )
}

export function PageLoader() {
  return (
    <div className="page-loader" role="status" aria-label="Lädt">
      <span className="lamp-pulse" />
    </div>
  )
}
