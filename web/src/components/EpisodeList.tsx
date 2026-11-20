import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router'
import { useI18n } from '../i18n'
import { api, playPath, type Item } from '../lib/api'
import { formatDate, isUpcoming } from '../lib/format'
import { ratio, remaining, useMarkEpisode, useProgress } from '../lib/progress'
import Icon from './Icon'
import { ErrorState } from './States'

/** `series` enables the "watched" toggle (it is stored with the mark). */
type Props = { tvId: number; season: number; current?: number; compact?: boolean; series?: Item }

export default function EpisodeList({ tvId, season, current, compact, series }: Props) {
  const q = useQuery({ queryKey: ['season', tvId, season], queryFn: () => api.season(tvId, season) })
  const progress = useProgress('tv', tvId)
  const mark = useMarkEpisode(tvId)
  const { t } = useI18n()

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
    return <p className="page-note">{t('ep.none')}</p>
  }

  return (
    <ol className={`episodes${compact ? ' is-compact' : ''}`}>
      {q.data.episodes.map((ep) => {
        const upcoming = isUpcoming(ep.air_date)
        const date = formatDate(ep.air_date)
        const active = current === ep.episode_number
        const p = progress.data?.find((r) => r.season === season && r.episode === ep.episode_number)
        const seen = !!p?.finished
        const started = !!p && !seen && p.position >= 30 && p.duration > 0
        const canMark = !!series && !upcoming
        return (
          <li key={ep.episode_number} className="episode-item">
            <Link
              to={playPath('tv', tvId, season, ep.episode_number)}
              className={`episode${active ? ' is-current' : ''}${upcoming ? ' is-upcoming' : ''}${seen ? ' is-seen' : ''}${canMark ? ' has-mark' : ''}`}
              aria-current={active ? 'true' : undefined}
              replace={compact}
            >
              <div className="episode-still">
                {ep.still ? <img src={ep.still} alt="" loading="lazy" decoding="async" /> : <Icon name="film" size={22} />}
                <span className="episode-play" aria-hidden="true">
                  <Icon name="play" size={18} />
                </span>
                {started && (
                  <span className="progress-bar" aria-hidden="true">
                    <span style={{ transform: `scaleX(${ratio(p.position, p.duration)})` }} />
                  </span>
                )}
              </div>
              <div className="episode-text">
                <h3 className="episode-name">
                  <span className="episode-no">{ep.episode_number}</span>
                  {ep.name}
                </h3>
                <p className="episode-meta">
                  {upcoming ? t('ep.airs', { date: date ?? t('ep.unknownDate') }) : date}
                  {ep.runtime ? `, ${t('fmt.m', { m: ep.runtime })}` : ''}
                  {seen && (
                    <span className="episode-state is-seen">
                      <Icon name="check" size={13} stroke={2.4} />
                      {t('ep.seen')}
                    </span>
                  )}
                  {started && <span className="episode-state">{remaining(p.position, p.duration)}</span>}
                </p>
                {!compact && ep.overview && <p className="episode-overview">{ep.overview}</p>}
              </div>
            </Link>
            {canMark && series && (
              <button
                type="button"
                className={`episode-mark${seen ? ' is-seen' : ''}`}
                aria-pressed={seen}
                aria-label={t(seen ? 'ep.markUnseenLabel' : 'ep.markSeenLabel', { n: ep.episode_number })}
                title={seen ? t('ep.unseen') : t('ep.markSeen')}
                onClick={() =>
                  mark.mutate({
                    s: season,
                    e: ep.episode_number,
                    watched: !seen,
                    meta: { item: series, episode_name: ep.name, still: ep.still },
                  })
                }
              >
                <Icon name="check" size={17} stroke={2.2} />
              </button>
            )}
          </li>
        )
      })}
    </ol>
  )
}
