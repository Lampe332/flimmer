import { useEffect, useRef } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useI18n } from '../../i18n'
import { ApiError } from '../../lib/api'
import { refreshTorrents, torrentApi, torrentLine, useTorrents, vpnProblem, type StartBody } from '../../lib/torrents'
import Icon from '../Icon'
import { Flicker } from '../States'
import { VpnBadge, VpnNotice } from '../Torrents'
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
  useEffect(() => {
    if (startBody.current) startTorrent(startBody.current)
  }, [p.hash, startTorrent])

  const begun = !p.startBody || start.isSuccess
  const info = useQuery({
    queryKey: ['torrent', p.hash],
    queryFn: () => torrentApi.get(p.hash),
    enabled: begun,
    refetchInterval: (q) => (q.state.data?.state === 'completed' ? 10_000 : 1000),
    retry: false,
  })
  const list = useTorrents()
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

  const status = tx && tx.state !== 'completed' ? torrentLine(tx) : undefined
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
          mode: play.data.mode,
          duration: play.data.duration,
          copyVideo: play.data.copy_video,
          audio: play.data.audio,
          status,
        }}
      />
    )
  }

  const problem = vpnProblem(vpn)
  const failed = start.error ?? (tx?.state === 'error' ? new Error(tx.error ?? t('tor.stError')) : null) ?? play.error
  return (
    <>
      {p.backdrop && <img className="stage-backdrop" src={p.backdrop} alt="" />}
      <div className="stage-card" role="status">
        {problem && !tx?.peers && tx?.state !== 'completed' ? (
          <VpnNotice vpn={vpn} />
        ) : failed ? (
          <>
            <span className="state-icon is-warn">
              <Icon name="warn" size={26} />
            </span>
            <h2>{t('tor.failedTitle')}</h2>
            <p>{failed instanceof ApiError && failed.status === 504 ? t('tor.noStartText') : t('tor.failedText')}</p>
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
