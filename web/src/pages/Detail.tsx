import { lazy, Suspense, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link, useParams, useSearchParams } from 'react-router'
import EpisodeList from '../components/EpisodeList'
import Icon from '../components/Icon'
import PosterCard from '../components/PosterCard'
import Row from '../components/Row'
import SaveButton from '../components/SaveButton'
import SeasonTabs from '../components/SeasonTabs'
import { ErrorState, PageLoader } from '../components/States'
import { useI18n } from '../i18n'
import { api, isMediaType, playPath, type MediaType } from '../lib/api'
import { torrentApi, torrentPlayPath } from '../lib/torrents'
import { formatRating, formatRuntime, typeLabel } from '../lib/format'
import { epCode, remaining, resumeTarget, useProgress } from '../lib/progress'
import { useTitle } from '../lib/title'
import NotFound from './NotFound'

const TrailerModal = lazy(() => import('../components/TrailerModal'))
const DownloadDialog = lazy(() => import('../components/DownloadDialog'))

export default function Detail() {
  const { type, id: idRaw } = useParams()
  const id = Number(idRaw)
  const valid = isMediaType(type) && Number.isInteger(id) && id > 0
  const [params, setParams] = useSearchParams()
  const [trailer, setTrailer] = useState(false)
  const [download, setDownload] = useState(false)
  const { t } = useI18n()

  const q = useQuery({
    queryKey: ['title', type, id],
    queryFn: () => api.title(type as 'movie' | 'tv', id),
    enabled: valid,
  })

  const progress = useProgress(type as 'movie' | 'tv', id, valid)
  useTitle(q.data?.title)
  // A finished download of this title (or of the episode "continue" points at) plays from the disk, offline.
  const loaded = q.data
  const localEp = loaded?.type === 'tv' ? (resumeTarget(progress.data ?? [], loaded.seasons) ?? { s: loaded.seasons[0]?.season_number ?? 1, e: 1 }) : null
  const local = useQuery({
    queryKey: ['local-file', type, id, localEp?.s, localEp?.e],
    queryFn: () => torrentApi.local(type as MediaType, id, localEp?.s, localEp?.e),
    enabled: valid && !!loaded,
    retry: false,
  })
  const localFile = local.data?.completed ? local.data : undefined
  const playCls = 'btn btn-primary btn-lg'
  // One play button: with a finished download it plays the file on disk (same episode the button names).
  const playTo = (stream: string) => (localFile ? torrentPlayPath(localFile.torrent_id, localFile.file) : stream)

  if (!valid) return <NotFound />
  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} />
  if (q.isPending) return <PageLoader />

  const d = q.data
  const rating = formatRating(d.rating)
  const runtime = formatRuntime(d.runtime)
  const firstSeason = d.seasons[0]?.season_number
  const rows = progress.data ?? []
  const movieRow = d.type === 'movie' ? rows[0] : undefined
  const movieResume = movieRow && !movieRow.finished && movieRow.position >= 30 ? movieRow : undefined
  const tvResume = d.type === 'tv' ? resumeTarget(rows, d.seasons) : null
  const tvStarted = tvResume?.row && !tvResume.row.finished && tvResume.row.position >= 30 ? tvResume.row : undefined
  const seasonParam = Number(params.get('staffel'))
  const hasSeason = (n: number | undefined) => d.seasons.some((s) => s.season_number === n)
  // Without a chosen season, open the one "continue watching" continues in.
  const season = hasSeason(seasonParam) ? seasonParam : hasSeason(tvResume?.s) ? tvResume?.s : firstSeason
  const browse = d.type === 'movie' ? '/filme' : '/serien'
  const seasonCount = d.seasons.length

  return (
    <article className="detail">
      <div className="detail-backdrop" aria-hidden="true">
        {d.backdrop && <img src={d.backdrop} alt="" fetchPriority="high" />}
        <div className="detail-shade" />
      </div>

      <div className="detail-head">
        <div className="detail-poster">
          {d.poster ? (
            <img src={d.poster} alt={t('detail.posterAlt', { title: d.title })} width={500} height={750} />
          ) : (
            <div className="card-noposter">
              <Icon name="film" size={40} />
            </div>
          )}
        </div>
        <div className="detail-info">
          <ul className="meta">
            <li className="meta-tag">{typeLabel(d.type)}</li>
            {d.year && <li>{d.year}</li>}
            {runtime && <li>{d.type === 'tv' ? t('detail.perEpisode', { runtime }) : runtime}</li>}
            {d.type === 'tv' && seasonCount > 0 && <li>{t('detail.seasons', { n: seasonCount })}</li>}
            {rating && (
              <li className="meta-rating">
                <Icon name="star" size={14} />
                {rating}
              </li>
            )}
          </ul>
          <h1 className="detail-title">{d.title}</h1>
          {d.original_title && d.original_title !== d.title && <p className="detail-original">{d.original_title}</p>}
          {d.tagline && <p className="detail-tagline">{d.tagline}</p>}
          <div className="actions">
            {tvResume ? (
              <Link to={playTo(playPath('tv', d.id, tvResume.s, tvResume.e))} className={playCls}>
                <Icon name="play" size={18} />
                {t('home.continue')} {epCode(tvResume.s, tvResume.e)}
                {tvStarted && <span className="btn-note">{remaining(tvStarted.position, tvStarted.duration)}</span>}
              </Link>
            ) : movieResume ? (
              <Link to={playTo(playPath('movie', d.id))} className={playCls}>
                <Icon name="play" size={18} />
                {t('home.continue')}
                <span className="btn-note">{remaining(movieResume.position, movieResume.duration)}</span>
              </Link>
            ) : (
              <Link to={playTo(playPath(d.type, d.id, firstSeason ?? 1, 1))} className={playCls}>
                <Icon name="play" size={18} />
                {d.type === 'tv' ? t('detail.playFirst') : t('common.play')}
              </Link>
            )}
            {d.trailer_youtube_key && (
              <button type="button" className="btn btn-glass btn-lg" onClick={() => setTrailer(true)}>
                <Icon name="film" size={18} />
                {t('detail.trailer')}
              </button>
            )}
            <SaveButton item={d} />
            {d.type === 'movie' && localFile ? (
              <button type="button" className="btn btn-glass btn-lg" disabled>
                <Icon name="check" size={18} />
                {t('detail.loaded')}
              </button>
            ) : (
              <button type="button" className="btn btn-glass btn-lg" onClick={() => setDownload(true)}>
                <Icon name="download" size={18} />
                {d.type === 'tv' && season !== undefined ? t('dl.seasonButton', { n: season }) : t('tor.download')}
              </button>
            )}
          </div>
          {d.overview ? (
            <p className="detail-overview">{d.overview}</p>
          ) : (
            <p className="detail-overview muted">{t('detail.noOverview')}</p>
          )}
          {d.genres.length > 0 && (
            <ul className="detail-genres" aria-label={t('detail.genres')}>
              {d.genres.map((g) => (
                <li key={g.id}>
                  <Link to={`${browse}?genre=${g.id}`} className="chip">
                    {g.name}
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      {d.type === 'tv' && season !== undefined && (
        <section className="detail-section" aria-label={t('detail.episodes')}>
          <div className="section-head">
            <h2 className="row-title">{t('detail.episodes')}</h2>
            <SeasonTabs
              seasons={d.seasons}
              value={season}
              onChange={(n) => setParams({ staffel: String(n) }, { replace: true })}
            />
          </div>
          <EpisodeList tvId={d.id} season={season} series={d} />
        </section>
      )}

      {d.cast.length > 0 && (
        <Row title={t('detail.cast')}>
          {d.cast.map((c) => (
            <figure key={`${c.name}-${c.character}`} className="person">
              <div className="person-img">
                {c.profile ? (
                  <img src={c.profile} alt="" loading="lazy" decoding="async" />
                ) : (
                  <span aria-hidden="true">{initials(c.name)}</span>
                )}
              </div>
              <figcaption>
                <span className="person-name">{c.name}</span>
                {c.character && <span className="person-role">{c.character}</span>}
              </figcaption>
            </figure>
          ))}
        </Row>
      )}

      {d.recommendations.length > 0 && (
        <Row title={t('detail.recommendations')}>
          {d.recommendations.map((r) => (
            <PosterCard key={`${r.type}-${r.id}`} item={r} />
          ))}
        </Row>
      )}

      {download && (
        <Suspense fallback={null}>
          <DownloadDialog d={d} season={season} onClose={() => setDownload(false)} />
        </Suspense>
      )}

      {trailer && d.trailer_youtube_key && (
        <Suspense fallback={null}>
          <TrailerModal youtubeKey={d.trailer_youtube_key} title={d.title} onClose={() => setTrailer(false)} />
        </Suspense>
      )}
    </article>
  )
}

function initials(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((p) => p[0]?.toUpperCase() ?? '')
    .join('')
}
