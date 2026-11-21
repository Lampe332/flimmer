import { Link } from 'react-router'
import { useI18n } from '../i18n'
import { IS_ANDROID_APP } from '../lib/platform'
import { Mark } from './Logo'

export default function Footer() {
  const { t } = useI18n()
  return (
    <footer className="footer">
      <div className="footer-inner">
        <Link to="/" className="footer-brand" aria-label={t('app.home')}>
          <Mark size={22} />
        </Link>
        <p className="footer-note">
          {t(IS_ANDROID_APP ? 'footer.beforeApp' : 'footer.before')}{' '}
          <a href="https://www.themoviedb.org/" target="_blank" rel="noreferrer noopener">
            TMDB
          </a>
          {t('footer.after')}
        </p>
        <nav className="footer-nav" aria-label={t('nav.footer')}>
          <Link to="/filme">{t('nav.movies')}</Link>
          <Link to="/serien">{t('nav.tv')}</Link>
          <Link to="/merkliste">{t('nav.watchlist')}</Link>
          <Link to="/suche">{t('nav.search')}</Link>
          <Link to="/einstellungen">{t('settings.title')}</Link>
          <Link to="/hilfe">{t('help.title')}</Link>
        </nav>
      </div>
    </footer>
  )
}
