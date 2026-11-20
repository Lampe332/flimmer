import type { ReactNode } from 'react'
import { Link, useLocation } from 'react-router'
import { ApiError } from '../lib/api'
import Icon, { type IconName } from './Icon'
import { Mark } from './Logo'

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const { pathname } = useLocation()
  const status = error instanceof ApiError ? error.status : -1
  let title = 'Das hat nicht geklappt'
  let text = error instanceof Error ? error.message : 'Unbekannter Fehler.'
  if (status === 0) {
    title = 'Server nicht erreichbar'
    text = 'Flimmer bekommt keine Antwort vom lokalen Server. Starte Flimmer neu und versuche es noch einmal.'
  } else if (status === 404) {
    title = 'Nicht gefunden'
    text = 'Diesen Titel gibt es bei TMDB nicht (mehr).'
  } else if (status >= 500) {
    title = 'TMDB antwortet gerade nicht'
    text = `${text}. Prüfe die Internetverbindung und den API-Schlüssel in config.json.`
  }
  return (
    <div className="state" role="alert">
      <span className="state-icon is-warn">
        <Icon name="warn" size={26} />
      </span>
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
          <Link to="/" className="btn btn-glass">
            Zur Startseite
          </Link>
        )}
      </div>
    </div>
  )
}

type EmptyProps = { title: string; text: string; icon?: IconName; children?: ReactNode }

export function EmptyState({ title, text, icon = 'film', children }: EmptyProps) {
  return (
    <div className="state">
      <span className="state-icon">
        <Icon name={icon} size={26} />
      </span>
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

/** The mark, flickering like a lamp that is warming up. */
export function Flicker({ size = 44, label }: { size?: number; label?: string }) {
  return (
    <span className="flicker" role={label ? 'status' : undefined} aria-label={label}>
      <Mark size={size} />
    </span>
  )
}

export function PageLoader() {
  return (
    <div className="page-loader">
      <Flicker label="Lädt" />
    </div>
  )
}
