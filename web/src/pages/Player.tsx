import { useEffect, useMemo, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate, useParams, useSearchParams } from 'react-router'
import EpisodeList from '../components/EpisodeList'
import Icon from '../components/Icon'
import ScanPanel from '../components/player/ScanPanel'
import VideoPlayer from '../components/player/VideoPlayer'
import SeasonTabs from '../components/SeasonTabs'
import { ErrorState, Flicker, PageLoader } from '../components/States'
import {
  api,
  ApiError,
  playPath,
  titlePath,
  isMediaType,
  type MediaType,
  type Scan,
  type ScanResult,
  type Settings,
} from '../lib/api'
import { useI18n } from '../i18n'
import { formatDate, formatHeight, isUpcoming } from '../lib/format'
import { rank } from '../lib/pick'
import { invalidateWatching, neighbour, resumeTarget, useProgress, type Ep } from '../lib/progress'
import { useTitle } from '../lib/title'
import NotFound from './NotFound'

/** Server defaults, used until the saved settings have loaded. */
const DEFAULTS: Settings = { filter: { min_height: 0, audio_lang: 'any', sort: 'quality' }, prefetch_minutes: 10 }
const NO_SCAN: Scan = { state: 'unavailable', started_at: null, finished_at: null, results: [] }
/** Seconds of scanning after which playback starts with the best source found so far. */
const EARLY_S = 8

export default function Player() {
  const { type, id: idRaw } = useParams()
  const [params] = useSearchParams()
  const id = Number(idRaw)
  if (!isMediaType(type) || !Number.isInteger(id) || id <= 0) return <NotFound />
  if (type === 'tv' && !params.has('s')) return <ResumeSeries id={id} />
  const s =Math.max(0, Math.floor(Number(params.get('s'))) || 1)
  const e = Math.max(1, Math.floor(Number(params.get('e'))) || 1)
  return <PlayerView key={`${type}-${id}-${s}-${e}`} type={type} id={id} s={s} e={e} />
}

/** `/play/tv/:id` without an episode: continue where you left off, else start with the first episode. */
function ResumeSeries({ id }: { id: number }) {
  const navigate = useNavigate()
  const details = useQuery({ queryKey: ['title', 'tv', id], queryFn: () => api.title('tv', id) })
  const progress = useProgress('tv', id)
  const ready = !!details.data && (progress.isFetched || progress.isError)
  useEffect(() => {
    if (!ready || !details.data) return
    const seasons = details.data.seasons
    const to = resumeTarget(progress.data ?? [], seasons)
    void navigate(playPath('tv', id, to?.s ?? seasons[0]?.season_number ?? 1, to?.e ?? 1), { replace: true })
  }, [ready, details.data, progress.data, id, navigate])
  if (details.error) return <ErrorState error={details.error} onRetry={() => void details.refetch()} />
  return <PageLoader />
}

function PlayerView({ type, id, s, e }: { type: MediaType; id: number; s: number; e: number }) {
  const qc = useQueryClient()
  const navigate = useNavigate()
  const isTv = type === 'tv'

  const [manual, setManual] = useState<string | null>(null)
  const [locked, setLocked] = useState<string | null>(null)
  const [dead, setDead] = useState<ReadonlySet<string>>(new Set())
  const [draft, setDraft] = useState<Settings | null>(null)
  const [viewSeason, setViewSeason] = useState(s)
  const resume = useRef(0)
  const [hintDone, setHintDone] = useState(false)
  const retried = useRef(new Set<string>())
  const { t } = useI18n()

  const details = useQuery({ queryKey: ['title', type, id], queryFn: () => api.title(type, id) })
  const season = useQuery({ queryKey: ['season', id, s], queryFn: () => api.season(id, s), enabled: isTv })
  const caps = useQuery({ queryKey: ['caps'], queryFn: api.caps, staleTime: 60_000, retry: false })
  const saved = useQuery({ queryKey: ['settings'], queryFn: api.settings, staleTime: Infinity, retry: false })
  const progress = useProgress(type, id)
  const sniffer = caps.data?.sniffer === true
  const settings = draft ?? saved.data ?? DEFAULTS

  const save = useMutation({
    mutationFn: api.settingsPut,
    onSuccess: (_void, next) => qc.setQueryData(['settings'], next),
  })

  // ---- scan: start it (app only), then poll while it runs ----
  const scanKey = useMemo(() => ['scan', type, id, s, e] as const, [type, id, s, e])
  const start = useMutation({
    mutationFn: (opts: { force?: boolean; provider?: string }) => api.scanStart(type, id, s, e, opts),
    onMutate: () => qc.cancelQueries({ queryKey: scanKey }),
    onSuccess: async (data) => {
      await qc.cancelQueries({ queryKey: scanKey })
      qc.setQueryData(scanKey, data)
    },
  })
  const startScan = start.mutate
  useEffect(() => {
    if (sniffer) startScan({})
  }, [sniffer, startScan])

  const scanQ = useQuery({
    queryKey: scanKey,
    queryFn: () => api.scan(type, id, s, e),
    enabled: caps.isFetched && (!sniffer || start.isSuccess || start.isError),
    refetchInterval: (q) => (q.state.data?.state === 'running' ? 1000 : false),
    staleTime: 0,
    gcTime: 0,
    retry: false,
  })
  // A server without the scan API (older build) behaves like one without a sniffer.
  const scanMissing = scanQ.error instanceof ApiError && scanQ.error.status === 404
  const scan: Scan | undefined = scanQ.data ?? (scanMissing ? NO_SCAN : undefined)
  const scanError = scanQ.error && !scanMissing ? scanQ.error : start.error

  const results: ScanResult[] = useMemo(
    () =>
      (scan?.results ?? []).map((r) =>
        dead.has(r.provider_id) ? { ...r, status: 'error', error: t('player.expired'), stream_id: null } : r,
      ),
    [scan, dead, t],
  )
  const shownScan = scan ? { ...scan, results } : undefined
  const ranked = useMemo(() => rank(results, settings.filter), [results, settings.filter])
  const running = scan?.state === 'running' || start.isPending
  const settled = !!scan && !running
  const best = ranked[0]
  const chosen = manual ?? locked
  const active = chosen ? results.find((r) => r.provider_id === chosen && r.status === 'ok' && r.stream_id) : undefined

  // Automatic choice: start quickly with the best result so far (after 8 s of scanning or when the scan is done),
  // then upgrade automatically while the scan is still running if a better one arrives. Manual choice always wins.
  const [early, setEarly] = useState(false)
  const [waited, setWaited] = useState(0)
  useEffect(() => {
    setEarly(false)
    setWaited(0)
    if (!running) return
    const t = setTimeout(() => setEarly(true), EARLY_S * 1000)
    const tick = setInterval(() => setWaited((w) => w + 1), 1000)
    return () => (clearTimeout(t), clearInterval(tick))
  }, [running, scanKey])
  useEffect(() => {
    if (!settled && !early) return
    if (chosen && !active) {
      setManual(null)
      setLocked(null)
    } else if (!chosen && best) {
      setLocked(best.provider_id)
    } else if (!manual && running && best && locked && best.provider_id !== locked) {
      setLocked(best.provider_id)
    }
  }, [settled, early, running, chosen, active, best, manual, locked])

  const onSettings = (next: Settings) => {
    setDraft(next)
    save.mutate(next)
    if (!manual) setLocked(null)
  }

  // The playing source broke for good: drop it, the automatic choice then takes the next best one.
  const onFailed = () => {
    const provider = active?.provider_id
    if (!provider) return
    setDead((old) => new Set(old).add(provider))
    setManual(null)
    setLocked(null)
  }
  const others = ranked.filter((r) => r.provider_id !== active?.provider_id).length
  const fallback =
    others > 0
      ? { label: t('player.otherSource'), run: onFailed }
      : sniffer
        ? {
            label: t('player.rescan'),
            run: () => {
              setDead(new Set())
              setManual(null)
              setLocked(null)
              startScan({ force: true })
            },
          }
        : undefined

  const onExpired = () => {
    const provider = active?.provider_id
    if (!provider) return
    setManual(null)
    setLocked(null)
    if (sniffer && !retried.current.has(provider)) {
      retried.current.add(provider)
      startScan({ force: true, provider })
    } else {
      setDead((old) => new Set(old).add(provider))
    }
  }

  const d = details.data
  useTitle(d?.title)
  const episode = season.data?.episodes.find((x) => x.episode_number === e)
  const seasons = d?.seasons ?? []
  const next = isTv ? neighbour(seasons, { s, e }, 1) : null
  const prev = isTv ? neighbour(seasons, { s, e }, -1) : null
  const nextSeason = useQuery({
    queryKey: ['season', id, next?.s],
    queryFn: () => api.season(id, next?.s ?? 1),
    enabled: isTv && !!next && next.s !== s,
  })
  const nextEp = next ? (next.s === s ? season.data : nextSeason.data)?.episodes.find((x) => x.episode_number === next.e) : undefined
  const nextPlayable = next && !isUpcoming(nextEp?.air_date ?? null) ? next : null

  // Resume from the saved position unless that movie/episode was finished.
  const savedPos = progress.data?.find((r) => (isTv ? r.season === s && r.episode === e : true))
  const savedStart = savedPos && !savedPos.finished && savedPos.position >= 5 ? savedPos.position : 0
  const progressReady = progress.isFetched || progress.isError
  const onSave = (position: number, duration: number) => {
    if (!d) return
    api
      .progressPut(type, id, s, e, { position, duration, item: d, episode_name: episode?.name ?? null, still: episode?.still ?? null })
      .then(() => invalidateWatching(qc, type, id))
      .catch(() => undefined)
  }
  const go = (to: Ep) => navigate(playPath('tv', id, to.s, to.e), { replace: true })
  const epLabel = isTv ? t('player.episodeLabel', { s, e }) : undefined
  const checked = results.filter((r) => r.status !== 'pending' && r.status !== 'running').length

  return (
    <div className="player-page">
      <div className="player-top">
        <Link
          to={titlePath({ type, id })}
          className="back-link"
          aria-label={t('player.back')}
          title={t('player.back')}
          onClick={(ev) => {
            // Came here from inside Flimmer: go back to exactly that page (and its scroll position).
            if (((window.history.state as { idx?: number } | null)?.idx ?? 0) > 0) {
              ev.preventDefault()
              void navigate(-1)
            }
          }}
        >
          <Icon name="left" size={18} />
        </Link>
        <div className="player-heading">
          <h1 className="player-title">{d ? d.title : 'Flimmer'}</h1>
          {isTv && (
            <p className="player-ep">
              {epLabel}
              {episode?.name && <span className="player-ep-name">{episode.name}</span>}
            </p>
          )}
        </div>
      </div>

      <div className="player-layout">
        <div className="player-main">
          {caps.isFetched && !sniffer && active && (
            <p className="app-note">
              <Icon name="app" size={18} />
              <span>
                <strong>{t('player.browserNoteStrong')}</strong> {t('player.browserNote')}
              </span>
            </p>
          )}

          <div className={`stage${active ? ' has-video' : ''}`}>
            {active?.stream_id && progressReady ? (
              <VideoPlayer
                key={active.stream_id}
                streamId={active.stream_id}
                kind={active.kind}
                variants={active.variants}
                title={d?.title ?? ''}
                subtitle={isTv ? [epLabel, episode?.name].filter(Boolean).join(', ') : undefined}
                startAt={resume.current || savedStart}
                onTime={(t) => (resume.current = t)}
                onExpired={onExpired}
                onSave={onSave}
                resumeHint={!hintDone && savedStart > 0 ? savedStart : undefined}
                onHintDone={() => setHintDone(true)}
                fallback={fallback}
                next={
                  nextPlayable
                    ? {
                        label: t('player.episodeLabel', { s: nextPlayable.s, e: nextPlayable.e }),
                        name: nextEp?.name,
                        still: nextEp?.still,
                        go: () => go(nextPlayable),
                      }
                    : undefined
                }
              />
            ) : (
              <>
                {d?.backdrop && <img className="stage-backdrop" src={d.backdrop} alt="" />}
                <div className="stage-card" role="status">
                  {scanError ? (
                    <ErrorState error={scanError} onRetry={() => (sniffer ? startScan({}) : void scanQ.refetch())} />
                  ) : !scan || !caps.isFetched ? (
                    <Flicker size={52} label={t('player.loadingSources')} />
                  ) : running ? (
                    <>
                      <Flicker size={52} />
                      <h2>{best ? t('player.found') : early ? t('player.stillSearching') : t('player.searching')}</h2>
                      <p>
                        {best
                          ? t('player.foundText', { n: Math.max(1, EARLY_S - waited) })
                          : early
                            ? t('player.earlyText')
                            : t('player.searchingText')}
                      </p>
                      {results.length > 0 && (
                        <span
                          className="scan-meter"
                          role="progressbar"
                          aria-label={t('player.meter')}
                          aria-valuemin={0}
                          aria-valuemax={results.length}
                          aria-valuenow={checked}
                          aria-valuetext={t('player.meterText', { n: checked, total: results.length })}
                        >
                          <span style={{ transform: `scaleX(${checked / results.length})` }} />
                        </span>
                      )}
                      {best && (
                        <button type="button" className="btn btn-primary" onClick={() => setLocked(best.provider_id)}>
                          <Icon name="play" size={16} />
                          {t('player.startNow')}
                          {formatHeight(best.max_height) && <span className="btn-note">{formatHeight(best.max_height)}</span>}
                        </button>
                      )}
                    </>
                  ) : !sniffer ? (
                    <>
                      <span className="state-icon">
                        <Icon name="app" size={26} />
                      </span>
                      <h2>{t('player.appOnly')}</h2>
                      <p>{t('player.appOnlyText')}</p>
                      <Link to={titlePath({ type, id })} className="btn btn-glass">
                        <Icon name="left" size={16} />
                        {t('player.backToTitle')}
                      </Link>
                    </>
                  ) : (
                    <>
                      <span className="state-icon is-warn">
                        <Icon name="warn" size={26} />
                      </span>
                      <h2>{t('player.none')}</h2>
                      <p>{results.length > 0 ? t('player.noneText') : t('player.noProviders')}</p>
                      <button type="button" className="btn btn-primary" onClick={() => startScan({ force: true })}>
                        <Icon name="refresh" size={16} />
                        {t('player.searchAgain')}
                      </button>
                    </>
                  )}
                </div>
              </>
            )}
          </div>

          {isTv && (
            <div className="ep-nav">
              <button type="button" className="btn btn-glass" disabled={!prev} onClick={() => prev && go(prev)}>
                <Icon name="left" size={16} />
                {t('player.prevEpisode')}
              </button>
              <button type="button" className="btn btn-glass" disabled={!next} onClick={() => next && go(next)}>
                {t('player.nextEpisode')}
                <Icon name="right" size={16} />
              </button>
            </div>
          )}

          <section className="now-info">
            {details.error ? null : d ? (
              <>
                {isTv && (
                  <h2 className="now-ep">
                    {episode ? `${e}. ${episode.name}` : t('player.episodeN', { e })}
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
        </div>

        <ScanPanel
          sniffer={sniffer}
          scan={shownScan}
          loading={!scan && !scanError}
          settings={settings}
          saveFailed={save.isError}
          onSettings={onSettings}
          active={active?.provider_id ?? null}
          manual={manual !== null}
          onPick={setManual}
          onAuto={() => (setManual(null), setLocked(null))}
          onRescan={() => (setManual(null), setLocked(null), startScan({ force: true }))}
        />
      </div>

      {isTv && seasons.length > 0 && (
        <section className="detail-section" aria-label={t('player.pickEpisode')}>
          <div className="section-head">
            <h2 className="row-title">{t('player.episodes')}</h2>
            <SeasonTabs seasons={seasons} value={viewSeason} onChange={setViewSeason} />
          </div>
          <EpisodeList tvId={id} season={viewSeason} current={viewSeason === s ? e : undefined} compact series={d} />
        </section>
      )}
    </div>
  )
}
