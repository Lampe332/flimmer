import { Link } from 'react-router'
import { Mark } from './Logo'

export default function Footer() {
  return (
    <footer className="footer">
      <div className="footer-inner">
        <Link to="/" className="footer-brand" aria-label="Flimmer, zur Startseite">
          <Mark size={22} />
        </Link>
        <p className="footer-note">
          Flimmer läuft lokal auf deinem Rechner. Titel, Bilder und Texte kommen von{' '}
          <a href="https://www.themoviedb.org/" target="_blank" rel="noreferrer noopener">
            TMDB
          </a>
          ; die Anwendung wird von TMDB weder unterstützt noch zertifiziert.
        </p>
        <nav className="footer-nav" aria-label="Fußnavigation">
          <Link to="/filme">Filme</Link>
          <Link to="/serien">Serien</Link>
          <Link to="/merkliste">Merkliste</Link>
          <Link to="/suche">Suche</Link>
        </nav>
      </div>
    </footer>
  )
}
