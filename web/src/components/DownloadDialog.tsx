import { useEffect, useRef } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router'
import { useI18n } from '../i18n'
import { api, type Details } from '../lib/api'
import { rankTorrents, refreshTorrents, torrentApi, useTorrents, type TorrentResult } from '../lib/torrents'
import Icon from './Icon'
import { TorrentRow, VpnNotice } from './Torrents'

const DEFAULT_FILTER = { min_height: 0, audio_lang: 'any', sort: 'quality' } as const

/** "Herunterladen" on the title page: torrents of the movie, or season packs of the chosen season. */
export default function DownloadDialog({ d, season, onClose }: { d: Details; season?: number; onClose: () => void }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const ref = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const el = ref.current
    if (!el) return
    el.showModal()
    return () => el.close()
  }, [])
  const tv = d.type === 'tv'
  const s = season ?? 1
  const found = useQuery({
    queryKey: ['torrents-for', d.type, d.id, s, 1],
    queryFn: () => torrentApi.forTitle(d.type, d.id, s, 1),
    staleTime: 5 * 60_000,
    retry: false,
  })
  const settings = useQuery({ queryKey: ['settings'], queryFn: api.settings, staleTime: Infinity, retry: false })
  const filter = settings.data?.filter ?? DEFAULT_FILTER
  const list = useTorrents()
  const listed = new Set(list.data?.torrents.map((x) => x.id) ?? [])
  const results = rankTorrents(found.data?.results ?? [], filter).filter((r) => !tv || r.release.pack)
  const start = useMutation({
    mutationFn: (r: TorrentResult) =>
      torrentApi.start({
        magnet: r.magnet,
        trackers: r.trackers,
        name: r.name,
        mode: 'download',
        // A pack without an episode in `media`: the server takes every video of the season.
        media: { type: d.type, id: d.id, title: d.title, year: d.year ?? undefined, season: tv ? s : undefined },
      }),
    onSuccess: () => refreshTorrents(qc),
  })
  const started = start.isSuccess

  return (
    <dialog
      ref={ref}
      className="modal modal-files"
      aria-labelledby="dl-dialog-title"
      onClose={onClose}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div className="modal-head">
        <h2 id="dl-dialog-title">{tv ? t('dl.dialogSeason', { title: d.title, n: s }) : t('dl.dialogMovie', { title: d.title })}</h2>
        <button type="button" className="icon-btn" onClick={onClose} aria-label={t('trailer.close')}>
          <Icon name="close" size={18} />
        </button>
      </div>
      <div className="dl-dialog">
        <p className="field-hint">{tv ? t('dl.dialogTvHint') : t('dl.dialogHint')}</p>
        <VpnNotice vpn={list.data?.vpn} compact />
        {found.isPending ? (
          <p className="ts-wait" role="status">
            <span className="bars" aria-hidden="true" />
            {t('tor.searching')}
          </p>
        ) : found.error ? (
          <p className="scan-empty">{t('tor.searchFailed')}</p>
        ) : results.length === 0 ? (
          <p className="scan-empty">{tv ? t('dl.noPack') : t('tor.none')}</p>
        ) : (
          <ol className="ts-list">
            {results.slice(0, 12).map((r) => (
              <TorrentRow
                key={r.infohash}
                r={r}
                filter={filter}
                inList={listed.has(r.infohash) || (start.isPending && start.variables?.infohash === r.infohash)}
                onDownload={() => start.mutate(r)}
              />
            ))}
          </ol>
        )}
        {start.isError && (
          <p className="field-error" role="alert">
            {t('tor.downloadFailed')}
          </p>
        )}
        {started && (
          <p className="set-saved" role="status">
            <Icon name="check" size={15} stroke={2.2} />
            {t('dl.started')}
            <Link to="/downloads" className="tgroup-link">
              {t('tor.toDownloads')}
            </Link>
          </p>
        )}
      </div>
    </dialog>
  )
}
