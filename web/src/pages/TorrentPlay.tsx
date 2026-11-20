import { useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate, useParams, useSearchParams } from 'react-router'
import Icon from '../components/Icon'
import TorrentStage from '../components/player/TorrentStage'
import { ErrorState, PageLoader } from '../components/States'
import { useI18n } from '../i18n'
import { api } from '../lib/api'
import { invalidateWatching, useProgress } from '../lib/progress'
import { torrentApi, torrentPlayPath, torrentTitle, type NextFile } from '../lib/torrents'
import { useTitle } from '../lib/title'
import NotFound from './NotFound'

/** `/play/torrent/:hash?file=N`: a torrent from the download list (or the free search) in the own player. */
export default function TorrentPlay() {
  const { hash = '' } = useParams()
  const [params] = useSearchParams()
  const fileRaw = params.get('file')
  const file = fileRaw === null ? undefined : Number(fileRaw)
  const valid = /^[0-9a-f]{40}$/.test(hash) && (file === undefined || (Number.isInteger(file) && file >= 0))
  if (!valid) return <NotFound />
  return <TorrentPlayView key={`${hash}-${file ?? ''}`} hash={hash} file={file} />
}

function TorrentPlayView({ hash, file }: { hash: string; file: number | undefined }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const navigate = useNavigate()
  const info = useQuery({ queryKey: ['torrent', hash], queryFn: () => torrentApi.get(hash), retry: false })
  const media = info.data?.media ?? null
  const mediaId = media?.id
  const details = useQuery({
    queryKey: ['title', media?.type, mediaId],
    queryFn: () => api.title(media?.type ?? 'movie', mediaId ?? 0),
    enabled: !!media && mediaId !== undefined,
  })
  const progress = useProgress(media?.type ?? 'movie', mediaId ?? 0, mediaId !== undefined)
  const resume = useRef(0)
  const [hintDone, setHintDone] = useState(false)
  /** The file that plays and the pack's next episode after it (from /playinfo). */
  const [playing, setPlaying] = useState<{ from: number; next: NextFile | null } | null>(null)
  const title = info.data ? torrentTitle(info.data) : ''
  useTitle(title || undefined)

  if (info.error) return <ErrorState error={info.error} onRetry={() => void info.refetch()} />
  if (!info.data || (mediaId !== undefined && !progress.isFetched && !progress.isError)) return <PageLoader />

  // A season pack's file knows its own episode; a single-episode torrent has it in `media`.
  const fileMeta = info.data.files.find((f) => f.index === (file ?? playing?.from))
  const s = fileMeta?.season ?? media?.season ?? 0
  const e = fileMeta?.episode ?? media?.episode ?? 0
  const isTv = media?.type === 'tv'
  const row = progress.data?.find((r) => (isTv ? r.season === s && r.episode === e : true))
  const savedStart = row && !row.finished && row.position >= 5 ? row.position : 0
  const d = details.data
  const onSave =
    media && mediaId !== undefined && d && (!isTv || e > 0)
      ? (position: number, duration: number) => {
          api
            .progressPut(media.type, mediaId, s, e, { position, duration, item: d })
            .then(() => invalidateWatching(qc, media.type, mediaId))
            .catch(() => undefined)
        }
      : undefined

  return (
    <div className="player-page">
      <div className="player-top">
        <Link
          to="/downloads"
          className="back-link"
          aria-label={t('player.back')}
          title={t('player.back')}
          onClick={(ev) => {
            if (((window.history.state as { idx?: number } | null)?.idx ?? 0) > 0) {
              ev.preventDefault()
              void navigate(-1)
            }
          }}
        >
          <Icon name="left" size={18} />
        </Link>
        <div className="player-heading">
          <h1 className="player-title">{title}</h1>
          {info.data.name !== title && <p className="player-ep">{info.data.name}</p>}
        </div>
      </div>
      <div className="stage has-video tplay-stage">
        <TorrentStage
          hash={hash}
          startBody={{ infohash: hash }}
          file={file}
          title={title}
          startAt={resume.current || savedStart}
          onTime={(x) => (resume.current = x)}
          onSave={onSave}
          resumeHint={!hintDone && savedStart > 0 ? savedStart : undefined}
          onHintDone={() => setHintDone(true)}
          onInfo={(p) => setPlaying({ from: p.file, next: p.next_file })}
          next={
            playing?.next
              ? {
                  label: playing.next.season && playing.next.episode ? t('player.episodeLabel', { s: playing.next.season, e: playing.next.episode }) : t('player.nextEpisode'),
                  go: () => {
                    const n = playing.next
                    if (!n) return
                    void torrentApi.next(hash, playing.from).catch(() => undefined)
                    void navigate(torrentPlayPath(hash, n.file), { replace: true })
                  },
                }
              : undefined
          }
          backdrop={d?.backdrop}
        />
      </div>
    </div>
  )
}
