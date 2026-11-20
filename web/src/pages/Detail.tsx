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
import { api, isMediaType, playPath } from '../lib/api'
import { formatRating, formatRuntime, typeLabel } from '../lib/format'
import { useTitle } from '../lib/title'
import NotFound from './NotFound'

const TrailerModal = lazy(() => import('../components/TrailerModal'))

export default function Detail() {
  const { type, id: idRaw } = useParams()
  const id = Number(idRaw)
  const valid = isMediaType(type) && Number.isInteger(id) && id > 0
  const [params, setParams] = useSearchParams()
  const [trailer, setTrailer] = useState(false)

  const q = useQuery({
    queryKey: ['title', type, id],
    queryFn: () => api.title(type as 'movie' | 'tv', id),
    enabled: valid,
  })

  useTitle(q.data?.title)

  if (!valid) return <NotFound />
  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} />
  if (q.isPending) return <PageLoader />

  const d = q.data
  const rating = formatRating(d.rating)
  const runtime = formatRuntime(d.runtime)
  const firstSeason = d.seasons[0]?.season_number
  const seasonParam = Number(params.get('staffel'))
  const season = d.seasons.some((s) => s.season_number === seasonParam) ? seasonParam : firstSeason
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
            <img src={d.poster} alt={`Poster von ${d.title}`} width={500} height={750} />
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
            {runtime && <li>{d.type === 'tv' ? `${runtime} pro Folge` : runtime}</li>}
            {d.type === 'tv' && seasonCount > 0 && <li>{seasonCount === 1 ? '1 Staffel' : `${seasonCount} Staffeln`}</li>}
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
            <Link to={playPath(d.type, d.id, firstSeason ?? 1, 1)} className="btn btn-primary btn-lg">
              <Icon name="play" size={18} />
              {d.type === 'tv' ? 'Ab Folge 1 abspielen' : 'Abspielen'}
            </Link>
            {d.trailer_youtube_key && (
              <button type="button" className="btn btn-glass btn-lg" onClick={() => setTrailer(true)}>
                <Icon name="film" size={18} />
                Trailer
              </button>
            )}
            <SaveButton item={d} />
          </div>
          {d.overview ? (
            <p className="detail-overview">{d.overview}</p>
          ) : (
            <p className="detail-overview muted">Keine Beschreibung auf Deutsch vorhanden.</p>
          )}
          {d.genres.length > 0 && (
            <ul className="detail-genres" aria-label="Genres">
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
        <section className="detail-section" aria-label="Episoden">
          <div className="section-head">
            <h2 className="row-title">Episoden</h2>
            <SeasonTabs
              seasons={d.seasons}
              value={season}
              onChange={(n) => setParams({ staffel: String(n) }, { replace: true })}
            />
          </div>
          <EpisodeList tvId={d.id} season={season} />
        </section>
      )}

      {d.cast.length > 0 && (
        <Row title="Besetzung">
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
        <Row title="Das könnte dir auch gefallen">
          {d.recommendations.map((r) => (
            <PosterCard key={`${r.type}-${r.id}`} item={r} />
          ))}
        </Row>
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
