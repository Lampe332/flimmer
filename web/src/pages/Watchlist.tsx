import { Link } from 'react-router'
import PosterCard from '../components/PosterCard'
import { CardSkeletons, EmptyState, ErrorState } from '../components/States'
import { useI18n } from '../i18n'
import { useTitle } from '../lib/title'
import { useWatchlist } from '../lib/watchlist'

export default function Watchlist() {
  const { t } = useI18n()
  const { data, isPending, error, refetch } = useWatchlist()
  useTitle(t('nav.watchlist'))

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1 className="page-title">{t('nav.watchlist')}</h1>
          {data && data.length > 0 && <p className="page-sub">{t('wl.count', { n: data.length })}</p>}
        </div>
      </div>
      {error ? (
        <ErrorState error={error} onRetry={() => void refetch()} />
      ) : isPending ? (
        <div className="grid">
          <CardSkeletons count={12} />
        </div>
      ) : data.length === 0 ? (
        <EmptyState icon="bookmark" title={t('wl.emptyTitle')} text={t('wl.emptyText')}>
          <Link to="/filme" className="btn btn-primary">
            {t('wl.discoverMovies')}
          </Link>
          <Link to="/serien" className="btn btn-glass">
            {t('wl.discoverTv')}
          </Link>
        </EmptyState>
      ) : (
        <div className="grid is-saved-grid">
          {data.map((item) => (
            <PosterCard key={`${item.type}-${item.id}`} item={item} showType />
          ))}
        </div>
      )}
    </div>
  )
}
