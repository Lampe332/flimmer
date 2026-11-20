import { useEffect, useMemo, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate, useParams, useSearchParams } from 'react-router'
import EpisodeList from '../components/EpisodeList'
import Icon from '../components/Icon'
import ScanPanel from '../components/player/ScanPanel'
import VideoPlayer from '../components/player/VideoPlayer'
import SeasonTabs from '../components/SeasonTabs'
import { ErrorState, Flicker } from '../components/States'
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
import { formatDate, formatHeight, isUpcoming } from '../lib/format'
import { rank } from '../lib/pick'
import { invalidateWatching, neighbour, useProgress, type Ep } from '../lib/progress'
import { useTitle } from '../lib/title'
import NotFound from './NotFound'

/** Server defaults, used until the saved settings have loaded. */
const DEFAULTS: Settings = { filter: { min_height: 0, audio_lang: 'any', sort: 'quality' }, prefetch_minutes: 10 }
const NO_SCAN: Scan = { state: 'unavailable', started_at: null, finished_at: null, results: [] }

export default function Player() {
  const { type, id: idRaw } = useParams()
  const [params] = useSearchParams()
  const id = Number(idRaw)
  if (!isMediaType(type) || !Number.isInteger(id) || id <= 0) return <NotFound />
  const s = Math.max(0, Math.floor(Number(params.get('s'))) || 1)
  const e = Math.max(1, Math.floor(Number(params.get('e'))) || 1)
  return <PlayerView key={`${type}-${id}-${s}-${e}`} type={type} id={id} s={s} e={e} />
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
        dead.has(r.provider_id) ? { ...r, status: 'error', error: 'Stream abgelaufen', stream_id: null } : r,
      ),
    [scan, dead],
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
  useEffect(() => {
    setEarly(false)
    if (!running) return
    const t = setTimeout(() => setEarly(true), 8000)
    return () => clearTimeout(t)
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
  const epLabel = isTv ? `Staffel ${s}, Folge ${e}` : undefined
  const checked = results.filter((r) => r.status !== 'pending' && r.status !== 'running').length

  return (
    <div className="player-page">
      <div className="player-top">
        <Link to={titlePath({ type, id })} className="back-link" aria-label={d ? `Zurück zu ${d.title}` : 'Zurück zum Titel'}>
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
                <strong>Quellen-Prüfung nur in der Flimmer-App.</strong> Im Browser spielt Flimmer bereits geprüfte Quellen ab,
                neue findet nur die App.
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
                next={
                  nextPlayable
                    ? {
                        label: `Staffel ${nextPlayable.s}, Folge ${nextPlayable.e}`,
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
                    <Flicker size={52} label="Quellen werden geladen" />
                  ) : running ? (
                    <>
                      <Flicker size={52} />
                      <h2>Quellen werden geprüft</h2>
                      <p>
                        {results.length > 0
                          ? `${checked} von ${results.length} Anbietern haben geantwortet. Flimmer startet mit der besten Quelle, sobald alle durch sind.`
                          : 'Flimmer fragt die Anbieter nach einem Stream für diesen Titel.'}
                      </p>
                      {best && (
                        <button type="button" className="btn btn-primary" onClick={() => setLocked(best.provider_id)}>
                          <Icon name="play" size={16} />
                          Jetzt mit {best.name} starten
                          {formatHeight(best.max_height) && <span className="btn-note">{formatHeight(best.max_height)}</span>}
                        </button>
                      )}
                    </>
                  ) : !sniffer ? (
                    <>
                      <span className="state-icon">
                        <Icon name="app" size={26} />
                      </span>
                      <h2>Quellen-Prüfung nur in der Flimmer-App</h2>
                      <p>
                        Im Browser kann Flimmer keine Streams aufspüren. Öffne diesen Titel in der Flimmer-App, dort werden
                        alle Quellen geprüft und die beste startet von selbst. Was die App schon gefunden hat, läuft danach
                        auch hier.
                      </p>
                      <Link to={titlePath({ type, id })} className="btn btn-glass">
                        <Icon name="left" size={16} />
                        Zurück zum Titel
                      </Link>
                    </>
                  ) : (
                    <>
                      <span className="state-icon is-warn">
                        <Icon name="warn" size={26} />
                      </span>
                      <h2>Keine Quelle spielbar</h2>
                      <p>
                        {results.length > 0
                          ? 'Kein Anbieter hat gerade einen Stream für diesen Titel geliefert. Das ändert sich oft nach ein paar Minuten.'
                          : 'Für diesen Titel sind keine Anbieter eingetragen.'}
                      </p>
                      <button type="button" className="btn btn-primary" onClick={() => startScan({ force: true })}>
                        <Icon name="refresh" size={16} />
                        Neu prüfen
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
                Vorherige Folge
              </button>
              <button type="button" className="btn btn-glass" disabled={!next} onClick={() => next && go(next)}>
                Nächste Folge
                <Icon name="right" size={16} />
              </button>
            </div>
          )}

          <section className="now-info">
            {details.error ? null : d ? (
              <>
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
        <section className="detail-section" aria-label="Folge wählen">
          <div className="section-head">
            <h2 className="row-title">Folgen</h2>
            <SeasonTabs seasons={seasons} value={viewSeason} onChange={setViewSeason} />
          </div>
          <EpisodeList tvId={id} season={viewSeason} current={viewSeason === s ? e : undefined} compact series={d} />
        </section>
      )}
    </div>
  )
}
