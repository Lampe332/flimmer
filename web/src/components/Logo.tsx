import { Link } from 'react-router'

/** Wordmark: the i's tittle is the projector lamp. */
export default function Logo() {
  return (
    <Link to="/" className="logo" aria-label="Flimmer, zur Startseite">
      <span aria-hidden="true">
        Fl<span className="logo-i">ı<span className="logo-lamp" /></span>mmer
      </span>
    </Link>
  )
}
