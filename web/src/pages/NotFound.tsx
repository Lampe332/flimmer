import { Link } from 'react-router'

export default function NotFound() {
  return (
    <div className="state notfound">
      <p className="notfound-code" aria-hidden="true">
        404
      </p>
      <h1>Hier läuft nichts</h1>
      <p>Diese Seite gibt es in Flimmer nicht. Vielleicht war der Link alt oder hat einen Tippfehler.</p>
      <div className="state-actions">
        <Link to="/" className="btn btn-primary">
          Zur Startseite
        </Link>
      </div>
    </div>
  )
}
