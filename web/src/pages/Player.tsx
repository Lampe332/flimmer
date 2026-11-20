import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate, useParams, useSearchParams } from 'react-router'
import EpisodeList from '../components/EpisodeList'
import Icon from '../components/Icon'
import SeasonTabs from '../components/SeasonTabs'
import { ErrorState } from '../components/States'
import { api, isMediaType, playPath, titlePath, type Season } from '../lib/api'
import { formatDate } from '../lib/format'
import { getLastSource, getPopupGuard, setLastSource, setPopupGuard } from '../lib/storage'
import NotFound from './NotFound'

const SANDBOX = 'allow-scripts allow-same-origin allow-forms allow-presentation'

type Ep = { s: number; e: number }

function neighbour(seasons: Season[], cur: Ep, dir: 1 | -1): Ep | null {
  const idx = seasons.findIndex((x) => x.season_number === cur.s)
  const here = seasons[idx]
  if (!here) return null
  if (dir === 1) {
    if (cur.e < here.episode_count) return { s: cur.s, e: cur.e + 1 }
    const next = seasons[idx + 1]
    return next && next.episode_count > 0 ? { s: next.season_number, e: 1 } : null
  }
  if (cur.e > 1) return { s: cur.s, e: cur.e - 1 }
  const prev = seasons[idx - 1]
  return prev && prev.episode_count > 0 ? { s: prev.season_number, e: prev.episode_count } : null
}

export default function Player() {
  const { type, id: idRaw } = useParams()
  const id = Number(idRaw)
  const valid = isMediaType(type) && Number.isInteger(id) && id > 0
  const mediaType = type === 'tv' ? 'tv' : 'movie'
  const [params] = useSearchParams()
  const navigate = useNavigate()
  const s = Math.max(0, Number(params.get('s')) || 1)
  const e = Math.max(1, Number(params.get('e')) || 1)
  const isTv = mediaType === 'tv'

  const [chosen, setChosen] = useState<string | null>(getLastSource)
  const [, setGuardVersion] = useState(0)
  const [viewSeason, setViewSeason] = useState(s)
  useEffect(() => setViewSeason(s), [s])

  const details = useQuery({
    queryKey: ['title', mediaType, id],
    queryFn: () => api.title(mediaType, id),
    enabled: valid,
  })
  const sources = useQuery({
    queryKey: ['sources', mediaType, id, isTv ? s : 0, isTv ? e : 0],
    queryFn: () => api.sources(mediaType, id, s, e),
    enabled: valid,
  })
  const season = useQuery({
    queryKey: ['season', id, s],
    queryFn: () => api.season(id, s),
    enabled: valid && isTv,
  })

  if (!valid) return <NotFound />

  const list = sources.data ?? []
  const source = list.find((x) => x.id === chosen) ?? list[0]
  const guard = source ? getPopupGuard(source.id) : true

  const d = details.data
  const episode = season.data?.episodes.find((x) => x.episode_number === e)
  const seasons = d?.seasons ?? []
  const next = isTv ? neighbour(seasons, { s, e }, 1) : null
  const prev = isTv ? neighbour(seasons, { s, e }, -1) : null
  const go = (to: Ep) => navigate(playPath('tv', id, to.s, to.e), { replace: true })

  const pick = (sid: string) => {
    setChosen(sid)
    setLastSource(sid)
  }
  const toggleGuard = () => {
    if (!source) return
    setPopupGuard(source.id, !guard)
    setGuardVersion((v) => v + 1)
  }

  return (
    <div className="player-page">
      <div className="player-top">
        <Link to={titlePath({ type: mediaType, id })} className="back-link">
          <Icon name="left" size={18} />
          <span className="back-text">{d ? d.title : 'Zurück zum Titel'}</span>
        </Link>
        {isTv && (
          <span className="player-ep-label">
            Staffel {s}, Folge {e}
          </span>
        )}
      </div>

      <div className="stage">
        {sources.error ? (
          <ErrorState error={sources.error} onRetry={() => void sources.refetch()} />
        ) : !source ? (
          <div className="frame frame-empty">
            {sources.isPending ? <span className="lamp-pulse" role="status" aria-label="Quellen laden" /> : <p>Für diesen Titel sind keine Quellen eingetragen.</p>}
          </div>
        ) : (
          <div className="frame">
            <iframe
              key={`${source.url}|${guard ? 'guard' : 'open'}`}
              src={source.url}
              title={`${d?.title ?? 'Video'} über ${source.name}`}
              allow="autoplay; fullscreen; encrypted-media; picture-in-picture"
              allowFullScreen
              referrerPolicy="origin"
              sandbox={guard ? SANDBOX : undefined}
            />
          </div>
        )}
      </div>

      <div className="controls">
        {isTv && (
          <div className="ep-nav">
            <button type="button" className="btn btn-ghost" disabled={!prev} onClick={() => prev && go(prev)}>
              <Icon name="left" size={18} />
              Vorherige Folge
            </button>
            <button type="button" className="btn btn-primary" disabled={!next} onClick={() => next && go(next)}>
              Nächste Folge
              <Icon name="right" size={18} />
            </button>
          </div>
        )}
        {source && (
          <div className="guard">
            <button
              type="button"
              role="switch"
              aria-checked={guard}
              className="switch"
              onClick={toggleGuard}
              aria-describedby="guard-hint"
            >
              <span className="switch-track" aria-hidden="true">
                <span className="switch-thumb" />
              </span>
              <Icon name="shield" size={16} />
              Popup-Schutz
            </button>
            <p id="guard-hint" className="guard-hint">
              {guard
                ? 'Blockiert Werbefenster. Lädt der Player nicht, schalte ihn für diese Quelle aus.'
                : `Aus für ${source.name}. Werbefenster können sich öffnen.`}
            </p>
          </div>
        )}
      </div>

      <section className="sources" aria-label="Quelle wählen">
        <h2 className="sources-title">Quelle</h2>
        <div className="source-list">
          {sources.isPending
            ? Array.from({ length: 8 }, (_, i) => <span key={i} className="chip skel chip-skel" aria-hidden="true" />)
            : list.map((x) => (
                <button
                  key={x.id}
                  type="button"
                  className={`chip source-chip${x.no_ads ? ' is-clean' : ''}`}
                  aria-pressed={x.id === source?.id}
                  onClick={() => pick(x.id)}
                  title={x.no_ads ? `${x.name}, laut Liste ohne Werbung` : x.name}
                >
                  {x.no_ads && <span className="clean-dot" aria-hidden="true" />}
                  {x.name}
                  {x.no_ads && <span className="sr-only">, ohne Werbung</span>}
                </button>
              ))}
        </div>
        {list.some((x) => x.no_ads) && (
          <p className="page-note">
            <span className="clean-dot" aria-hidden="true" /> laut Anbieterliste ohne Werbung
          </p>
        )}
      </section>

      <section className="now-info">
        {details.error ? null : d ? (
          <>
            <h1 className="now-title">{d.title}</h1>
            {isTv && (
              <h2 className="now-ep">
                {episode ? `${e}. ${episode.name}` : `Folge ${e}`}
                {episode?.air_date && <span className="now-date">{formatDate(episode.air_date)}</span>}
              </h2>
            )}
            <p className="now-overview">{(isTv && episode?.overview) || d.overview}</p>
          </>
        ) : (
          <>
            <div className="skel skel-title" />
            <div className="skel skel-line" />
          </>
        )}
      </section>

      {isTv && seasons.length > 0 && (
        <section className="detail-section" aria-label="Folge wählen">
          <div className="section-head">
            <h2 className="row-title">Folgen</h2>
          </div>
          <SeasonTabs seasons={seasons} value={viewSeason} onChange={setViewSeason} />
          <EpisodeList tvId={id} season={viewSeason} current={viewSeason === s ? e : undefined} compact />
        </section>
      )}
    </div>
  )
}
