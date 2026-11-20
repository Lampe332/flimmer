import { Component, type ReactNode } from 'react'
import Icon from './Icon'

type Props = { children: ReactNode }
type State = { error: Error | null }

/** Catches render errors and failed lazy chunk loads (e.g. server stopped while the app was open). */
export default class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null }

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  render() {
    const { error } = this.state
    if (!error) return this.props.children
    const chunk = /dynamically imported module|Failed to fetch|Importing a module script failed/i.test(error.message)
    return (
      <div className="state" role="alert">
        <span className="state-icon is-warn">
          <Icon name="warn" size={26} />
        </span>
        <h2>{chunk ? 'Server nicht erreichbar' : 'Diese Seite ist abgestürzt'}</h2>
        <p>
          {chunk
            ? 'Flimmer konnte diese Seite nicht vom lokalen Server laden. Starte Flimmer neu und lade die Seite noch einmal.'
            : error.message}
        </p>
        <div className="state-actions">
          <button type="button" className="btn btn-primary" onClick={() => window.location.reload()}>
            <Icon name="refresh" size={18} />
            Neu laden
          </button>
        </div>
      </div>
    )
  }
}
