import type { ReactNode } from 'react'
import { Link, useLocation } from 'react-router'
import { useI18n } from '../i18n'
import { ApiError } from '../lib/api'
import { errorText } from '../lib/errors'
import Icon, { type IconName } from './Icon'
import { Mark } from './Logo'

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const { pathname } = useLocation()
  const { t } = useI18n()
  const status = error instanceof ApiError ? error.status : -1
  let title = t('err.generic')
  // The server's message is English and for the log; the text comes from its reason code.
  let text = errorText(error)
  if (status === 0) {
    title = t('err.serverDown')
  } else if (status === 404) {
    title = t('err.notFound')
    text = t('err.notFoundText')
  } else if (status >= 500) {
    title = t('err.tmdbDown')
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
            {t('err.retry')}
          </button>
        )}
        {pathname !== '/' && (
          <Link to="/" className="btn btn-glass">
            {t('common.toHome')}
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
  const { t } = useI18n()
  return (
    <div className="page-loader">
      <Flicker label={t('common.loading')} />
    </div>
  )
}
