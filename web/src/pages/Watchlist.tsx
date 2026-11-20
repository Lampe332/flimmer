import { Link } from 'react-router'
import PosterCard from '../components/PosterCard'
import { CardSkeletons, EmptyState, ErrorState } from '../components/States'
import { useWatchlist } from '../lib/watchlist'

export default function Watchlist() {
  const { data, isPending, error, refetch } = useWatchlist()

  return (
    <div className="page">
      <div className="page-head">
        <h1 className="page-title">Merkliste</h1>
        {data && data.length > 0 && <p className="page-count">{data.length === 1 ? '1 Titel' : `${data.length} Titel`}</p>}
      </div>
      {error ? (
        <ErrorState error={error} onRetry={() => void refetch()} />
      ) : isPending ? (
        <div className="grid">
          <CardSkeletons count={12} />
        </div>
      ) : data.length === 0 ? (
        <EmptyState title="Deine Merkliste ist leer" text="Merk dir Filme und Serien mit dem Plus auf einem Poster oder mit „Merken“ auf der Titelseite.">
          <Link to="/filme" className="btn btn-primary">
            Filme entdecken
          </Link>
          <Link to="/serien" className="btn btn-ghost">
            Serien entdecken
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
