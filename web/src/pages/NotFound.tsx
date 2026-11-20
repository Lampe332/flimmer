import { Link } from 'react-router'
import { useI18n } from '../i18n'

export default function NotFound() {
  const { t } = useI18n()
  return (
    <div className="state notfound">
      <p className="notfound-code" aria-hidden="true">
        404
      </p>
      <h1>{t('nf.title')}</h1>
      <p>{t('nf.text')}</p>
      <div className="state-actions">
        <Link to="/" className="btn btn-primary">
          {t('common.toHome')}
        </Link>
        <Link to="/suche" className="btn btn-glass">
          {t('nf.search')}
        </Link>
      </div>
    </div>
  )
}
