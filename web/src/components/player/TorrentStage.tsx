import { useEffect, useRef } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useI18n } from '../../i18n'
import { ApiError } from '../../lib/api'
import { codeText } from '../../lib/errors'
import { refreshTorrents, torrentApi, torrentLine, useTorrents, vpnProblem, type PlayInfo, type StartBody } from '../../lib/torrents'
import Icon from '../Icon'
import { Flicker } from '../States'
import { NeedsVpn, VpnBadge, VpnNotice } from '../Torrents'
import VideoPlayer from './VideoPlayer'

type Props = {
  hash: string
  /** Start (or resume) the torrent first; without it the torrent must already be in the list. */
  startBody?: StartBody
  file?: number
  title: string
  subtitle?: string
  startAt: number
  onTime: (seconds: number) => void
  onSave?: (seconds: number, duration: number) => void
  /** The file's /playinfo once it is there (next file of a season pack, tracks). */
  onInfo?: (info: PlayInfo) => void
  resumeHint?: number
  onHintDone?: () => void
  fallback?: { label: string; run: () => void }
  next?: { label: string; name?: string; still?: string | null; go: () => void }
  /** Backdrop behind the waiting card. */
  backdrop?: string | null
}

/**
 * Plays a torrent in the own player: starts it in stream mode, waits for the file's start (/playinfo), then hands
 * the player a direct or transcoded source. Leaving pauses a stream that was not turned into a download.
 */
export default function TorrentStage(p: Props) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const start = useMutation({ mutationFn: torrentApi.start, onSuccess: () => refreshTorrents(qc) })
  const startBody = useRef(p.startBody)
  const startTorrent = start.mutate
  const list = useTorrents()
  // A file that is already complete on disk plays as it is: no start, so it never needs the VPN.
  const listed = list.data?.torrents.find((x) => x.id.toLowerCase() === p.hash.toLowerCase())
  const done =
    listed?.state === 'completed' &&
    (p.file === undefined || listed.files.some((f) => f.index === p.file && f.size > 0 && f.downloaded >= f.size))
  const decided = list.isFetched
  useEffect(() => {
    if (startBody.current && decided && !done) startTorrent(startBody.current)
    // `done` only decides the first time; a stream that finishes while playing is not started again.
  }, [p.hash, startTorrent, decided])

  const begun = !p.startBody || done || start.isSuccess
  const info = useQuery({
    queryKey: ['torrent', p.hash],
    queryFn: () => torrentApi.get(p.hash),
    enabled: begun,
    refetchInterval: (q) => (q.state.data?.state === 'completed' ? 10_000 : 1000),
    retry: false,
  })
  const vpn = list.data?.vpn
  const tx = info.data ?? start.data
  const hasFiles = (tx?.files.length ?? 0) > 0
  const play = useQuery({
    queryKey: ['playinfo', p.hash, p.file ?? null],
    queryFn: () => torrentApi.playinfo(p.hash, p.file),
    enabled: begun && hasFiles,
    staleTime: Infinity,
    // 504: the file's start did not arrive within 90 s; keep waiting a little longer while peers trickle in.
    retry: (n, err) => err instanceof ApiError && err.status === 504 && n < 3,
  })

  const onInfo = useRef(p.onInfo)
  onInfo.current = p.onInfo
  useEffect(() => {
    if (play.data) onInfo.current?.(play.data)
  }, [play.data])

  // Leaving: a pure stream stops loading; a download (or a finished file) stays as it is.
  const latest = useRef(tx)
  latest.current = tx
  useEffect(
    () => () => {
      const x = latest.current
      if (x && x.mode === 'stream' && x.state !== 'completed' && x.state !== 'paused') {
        void torrentApi.pause(x.id).catch(() => undefined)
        refreshTorrents(qc)
      }
    },
    [qc],
  )

  const external = useMutation({ mutationFn: (file: number) => torrentApi.external(p.hash, file) })
  const status = tx && tx.state !== 'completed' ? torrentLine(tx) : undefined
  if (play.data?.mode === 'external') {
    const file = play.data.file
    return (
      <>
        {p.backdrop && <img className="stage-backdrop" src={p.backdrop} alt="" />}
        <div className="stage-card" role="status">
          <span className="state-icon is-warn">
            <Icon name="warn" size={26} />
          </span>
          <h2>{t('tor.externalTitle')}</h2>
          <p>{external.isError ? t('tor.externalNone') : t('tor.externalText')}</p>
          <button type="button" className="btn btn-primary" disabled={external.isPending} onClick={() => external.mutate(file)}>
            <Icon name="play" size={16} solid />
            {t('tor.externalOpen')}
          </button>
          {status && <p className="stage-hint">{status}</p>}
        </div>
      </>
    )
  }
  if (play.data) {
    return (
      <VideoPlayer
        key={`${p.hash}-${play.data.file}`}
        streamId={`torrent-${p.hash}-${play.data.file}`}
        kind="mp4"
        variants={[]}
        title={p.title}
        subtitle={p.subtitle}
        startAt={p.startAt}
        onTime={p.onTime}
        onExpired={() => undefined}
        onSave={p.onSave}
        resumeHint={p.resumeHint}
        onHintDone={p.onHintDone}
        fallback={p.fallback}
        next={p.next}
        torrent={{
          id: p.hash,
          file: play.data.file,
          mode: play.data.mode === 'transcode' ? 'transcode' : 'direct',
          duration: play.data.duration,
          copyVideo: play.data.copy_video,
          audio: play.data.audio,
          defaultAudio: play.data.default_audio,
          subtitles: play.data.subtitles,
          status,
        }}
      />
    )
  }

  const problem = vpnProblem(vpn)
  const failed = start.error ?? (tx?.state === 'error' ? new Error(t('tor.stError')) : null) ?? play.error
  return (
    <>
      {p.backdrop && <img className="stage-backdrop" src={p.backdrop} alt="" />}
      <div className="stage-card" role="status">
        {problem && !tx?.peers && tx?.state !== 'completed' ? (
          <VpnNotice vpn={vpn} />
        ) : failed instanceof ApiError && failed.code === 'needs_vpn' ? (
          <NeedsVpn />
        ) : failed ? (
          <>
            <span className="state-icon is-warn">
              <Icon name="warn" size={26} />
            </span>
            <h2>{t('tor.failedTitle')}</h2>
            <p>
              {failed instanceof ApiError && failed.status === 504
                ? t('tor.slowStartText')
                : tx?.state === 'error' && tx.error_code
                    ? codeText(tx.error_code)
                    : t('tor.noDataText')}
            </p>
            {p.fallback && (
              <button type="button" className="btn btn-primary" onClick={p.fallback.run}>
                <Icon name="refresh" size={16} />
                {p.fallback.label}
              </button>
            )}
          </>
        ) : (
          <>
            <Flicker size={52} />
            <h2>{t('tor.preparing')}</h2>
            <p>{tx ? torrentLine(tx) : t('tor.stStarting')}</p>
            <p className="stage-hint">{t('tor.preparingHint')}</p>
            <VpnBadge vpn={vpn} />
          </>
        )}
      </div>
    </>
  )
}
