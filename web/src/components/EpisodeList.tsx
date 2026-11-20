import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router'
import { api, playPath } from '../lib/api'
import { formatDate, isUpcoming } from '../lib/format'
import Icon from './Icon'
import { ErrorState } from './States'

type Props = { tvId: number; season: number; current?: number; compact?: boolean }

export default function EpisodeList({ tvId, season, current, compact }: Props) {
  const q = useQuery({ queryKey: ['season', tvId, season], queryFn: () => api.season(tvId, season) })

  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} />

  if (q.isPending) {
    return (
      <ol className={`episodes${compact ? ' is-compact' : ''}`} aria-busy="true">
        {Array.from({ length: compact ? 4 : 6 }, (_, i) => (
          <li key={i} className="episode episode-skel" aria-hidden="true">
            <div className="episode-still skel" />
            <div className="episode-text">
              <div className="skel skel-line short" />
              <div className="skel skel-line" />
            </div>
          </li>
        ))}
      </ol>
    )
  }

  if (q.data.episodes.length === 0) {
    return <p className="page-note">Für diese Staffel sind noch keine Folgen eingetragen.</p>
  }

  return (
    <ol className={`episodes${compact ? ' is-compact' : ''}`}>
      {q.data.episodes.map((ep) => {
        const upcoming = isUpcoming(ep.air_date)
        const date = formatDate(ep.air_date)
        const active = current === ep.episode_number
        return (
          <li key={ep.episode_number}>
            <Link
              to={playPath('tv', tvId, season, ep.episode_number)}
              className={`episode${active ? ' is-current' : ''}${upcoming ? ' is-upcoming' : ''}`}
              aria-current={active ? 'true' : undefined}
              replace={compact}
            >
              <div className="episode-still">
                {ep.still ? <img src={ep.still} alt="" loading="lazy" decoding="async" /> : <Icon name="film" size={22} />}
                <span className="episode-play" aria-hidden="true">
                  <Icon name="play" size={18} />
                </span>
              </div>
              <div className="episode-text">
                <h3 className="episode-name">
                  <span className="episode-no">{ep.episode_number}</span>
                  {ep.name}
                </h3>
                <p className="episode-meta">
                  {upcoming ? `Erscheint am ${date ?? 'unbekannten Datum'}` : date}
                  {ep.runtime ? `, ${ep.runtime} Min.` : ''}
                </p>
                {!compact && ep.overview && <p className="episode-overview">{ep.overview}</p>}
              </div>
            </Link>
          </li>
        )
      })}
    </ol>
  )
}
