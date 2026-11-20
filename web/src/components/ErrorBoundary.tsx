import { Component, type ReactNode } from 'react'
import { t } from '../i18n'
import Icon from './Icon'

type Props = { children: ReactNode }
type State = { error: Error | null }

/** Catches render errors and failed lazy chunk loads (e.g. server stopped while the app was open). */
export default class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null }

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  // Texts come from the current language; App re-renders this boundary when it changes.
  render() {
    const { error } = this.state
    if (!error) return this.props.children
    const chunk = /dynamically imported module|Failed to fetch|Importing a module script failed/i.test(error.message)
    return (
      <div className="state" role="alert">
        <span className="state-icon is-warn">
          <Icon name="warn" size={26} />
        </span>
        <h2>{chunk ? t('err.serverDown') : t('err.crashed')}</h2>
        <p>{chunk ? t('err.chunk') : error.message}</p>
        <div className="state-actions">
          <button type="button" className="btn btn-primary" onClick={() => window.location.reload()}>
            <Icon name="refresh" size={18} />
            {t('err.reload')}
          </button>
        </div>
      </div>
    )
  }
}
