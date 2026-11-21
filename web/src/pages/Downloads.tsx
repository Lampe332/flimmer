import { useEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from 'react'
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate, useSearchParams } from 'react-router'
import AutoRules from '../components/AutoRules'
import Icon from '../components/Icon'
import { VpnImport } from '../components/SetupParts'
import { EmptyState, ErrorState, PageLoader } from '../components/States'
import { activeCount, FilterBar, FilterEmpty, useTorrentFilter } from '../components/TorrentFilters'
import Sheet from '../components/Sheet'
import { useIsPhone } from '../lib/platform'
import { EMPTY_FILTER } from '../lib/prefs'
import { DirectSwitch, QueueFields } from '../components/TorrentSettingsParts'
import { NeedsVpn, TorrentRow, VpnBadge, VpnNotice } from '../components/Torrents'
import { useI18n, type Key } from '../i18n'
import { api, ApiError } from '../lib/api'
import { codeText, errorText } from '../lib/errors'
import {
  episodeOfFile,
  formatBytes,
  formatEta,
  isActive,
  refreshTorrents,
  sourceName,
  torrentApi,
  torrentLine,
  torrentPlayPath,
  torrentTitle,
  useTorrents,
  type Torrent,
  type TorrentResult,
  type TorrentSettings,
  type Vpn,
} from '../lib/torrents'
import { useRole } from '../lib/session'
import { useTitle } from '../lib/title'

type Tab = 'downloads' | 'suche' | 'automatisch' | 'einstellungen'
/** `short`: the word under the icon on a phone, where four full labels do not fit next to each other. */
const TABS: { id: Tab; label: Key; short: Key; icon: 'download' | 'search' | 'sliders' | 'clock' }[] = [
  { id: 'downloads', label: 'dl.tabList', short: 'dl.tabListShort', icon: 'download' },
  { id: 'suche', label: 'dl.tabSearch', short: 'dl.tabSearchShort', icon: 'search' },
  { id: 'automatisch', label: 'dl.tabAuto', short: 'dl.tabAutoShort', icon: 'clock' },
  { id: 'einstellungen', label: 'dl.tabSettings', short: 'dl.tabSettingsShort', icon: 'sliders' },
]

export default function Downloads() {
  const { t } = useI18n()
  const phone = useIsPhone()
  const [params, setParams] = useSearchParams()
  const raw = params.get('tab')
  // Folders, VPN and auto-download rules belong to the PC: a paired phone gets the list and the search.
  const device = useRole() === 'device'
  const tabs = device ? TABS.filter((x) => x.id === 'downloads' || x.id === 'suche') : TABS
  const wanted: Tab = raw === 'suche' || raw === 'einstellungen' || raw === 'automatisch' ? raw : 'downloads'
  const tab: Tab = tabs.some((x) => x.id === wanted) ? wanted : 'downloads'
  useTitle(t(TABS.find((x) => x.id === tab)?.label ?? 'dl.tabList'))
  const list = useTorrents()
  const torrents = list.data?.torrents ?? []
  const loading = torrents.filter(isActive).length
  const done = torrents.filter((x) => x.state === 'completed').length

  return (
    <div className="page downloads">
      <div className="page-head">
        <div>
          <h1 className="page-title">{t('nav.downloads')}</h1>
          <p className="page-sub">
            {torrents.length === 0 ? t('dl.sub') : [loading > 0 && t('dl.loadingN', { n: loading }), done > 0 && t('dl.doneN', { n: done })].filter(Boolean).join(' · ') || t('dl.sub')}
          </p>
        </div>
        <VpnBadge vpn={list.data?.vpn} always />
      </div>

      <div className="tabs dl-tabs" role="tablist" aria-label={t('nav.downloads')}>
        {tabs.map((x) => (
          <button
            key={x.id}
            type="button"
            role="tab"
            id={`dl-tab-${x.id}`}
            aria-selected={tab === x.id}
            aria-controls={`dl-panel-${x.id}`}
            className="tab"
            onClick={() => {
              // The search keeps its words across tabs, so its results are still there on coming back.
              const q = params.get('q')
              setParams({ ...(x.id === 'downloads' ? {} : { tab: x.id }), ...(q ? { q } : {}) }, { replace: true })
            }}
          >
            <Icon name={x.icon} size={16} />
            {phone ? (
              <span className="tab-word">
                {t(x.short)}
                {x.id === 'downloads' && torrents.length > 0 && <span className="tab-count"> {torrents.length}</span>}
              </span>
            ) : (
              <>
                {t(x.label)}
                {x.id === 'downloads' && torrents.length > 0 && <span className="tab-count">{torrents.length}</span>}
              </>
            )}
          </button>
        ))}
      </div>

      <div role="tabpanel" id={`dl-panel-${tab}`} aria-labelledby={`dl-tab-${tab}`} className="dl-panel">
        {tab === 'downloads' ? (
          list.error ? (
            <ErrorState error={list.error} onRetry={() => void list.refetch()} />
          ) : list.isPending ? (
            <PageLoader />
          ) : (
            <DownloadList torrents={torrents} vpn={list.data?.vpn} onSearch={() => setParams({ tab: 'suche' }, { replace: true })} />
          )
        ) : tab === 'suche' ? (
          <FreeSearch torrents={torrents} />
        ) : tab === 'automatisch' ? (
          <AutoRules />
        ) : (
          <SettingsForm vpn={list.data?.vpn} />
        )}
      </div>
    </div>
  )
}

// ---------------------------------------------------------------- list

/** Downloads still to load, in the order the queue serves them: priority first, then age (like the server). */
function queueOrder(torrents: Torrent[]): Torrent[] {
  return torrents
    .filter((x) => x.mode === 'download' && x.state !== 'completed' && x.state !== 'paused')
    .sort((a, z) => z.priority - a.priority || a.added_at.localeCompare(z.added_at))
}

function DownloadList({ torrents, vpn, onSearch }: { torrents: Torrent[]; vpn: Vpn | undefined; onSearch: () => void }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const [removing, setRemoving] = useState<Torrent | null>(null)
  const [picking, setPicking] = useState<Torrent | null>(null)
  const queue = queueOrder(torrents)
  // Moving one step: the whole queue gets fresh priorities in the new order (top = highest).
  const move = useMutation({
    mutationFn: async ({ id, by }: { id: string; by: -1 | 1 }) => {
      const list = [...queue]
      const i = list.findIndex((x) => x.id === id)
      const j = i + by
      if (i < 0 || j < 0 || j >= list.length) return
      const [a, b] = [list[i], list[j]]
      if (!a || !b) return
      list[i] = b
      list[j] = a
      const top = Math.min(100, list.length * 2)
      await Promise.all(
        list.map((x, k) => {
          const p = top - k * 2
          return x.priority === p ? undefined : torrentApi.priority(x.id, p)
        }),
      )
    },
    onSettled: () => refreshTorrents(qc),
  })
  if (torrents.length === 0)
    return (
      <EmptyState icon="download" title={t('dl.emptyTitle')} text={t('dl.emptyText')}>
        <button type="button" className="btn btn-primary" onClick={onSearch}>
          <Icon name="search" size={17} />
          {t('dl.tabSearch')}
        </button>
      </EmptyState>
    )
  const rest = torrents.filter((x) => !queue.includes(x))
  const card = (x: Torrent, pos?: number) => (
    <DownloadCard
      key={x.id}
      x={x}
      onRemove={() => setRemoving(x)}
      onPick={() => setPicking(x)}
      queue={
        pos === undefined
          ? undefined
          : {
              pos,
              first: pos === 0,
              last: pos === queue.length - 1,
              busy: move.isPending,
              up: () => move.mutate({ id: x.id, by: -1 }),
              down: () => move.mutate({ id: x.id, by: 1 }),
            }
      }
    />
  )
  return (
    <>
      {torrents.some((x) => x.state !== 'completed') && <VpnNotice vpn={vpn} />}
      <MaxActive running={queue.filter(isActive).length} waiting={queue.filter((x) => x.state === 'queued').length} />
      {queue.length > 0 && (
        <>
          <h2 className="dl-section">{t('dl.queueTitle')}</h2>
          <ol className="dl-list">{queue.map((x, i) => card(x, i))}</ol>
        </>
      )}
      {rest.length > 0 && (
        <>
          {queue.length > 0 && <h2 className="dl-section">{t('dl.otherTitle')}</h2>}
          <ol className="dl-list">{rest.map((x) => card(x))}</ol>
        </>
      )}
      {move.isError && (
        <p className="field-error" role="alert">
          {t('dl.moveFailed')}
        </p>
      )}
      {removing && <RemoveDialog x={removing} onClose={() => setRemoving(null)} />}
      {picking && <FilePicker x={picking} onClose={() => setPicking(null)} />}
    </>
  )
}

/** "Gleichzeitig laden: 1 … 10": the server's queue reads it every second, so it acts at once. */
function MaxActive({ running, waiting }: { running: number; waiting: number }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const saved = useQuery({ queryKey: ['torrent-settings'], queryFn: torrentApi.settings, retry: false })
  const save = useMutation({
    mutationFn: torrentApi.settingsPut,
    onSuccess: (_v, next) => {
      qc.setQueryData(['torrent-settings'], next)
      refreshTorrents(qc)
    },
  })
  const s = saved.data
  if (!s) return null
  const set = (n: number) => save.mutate({ ...s, max_active: Math.max(1, Math.min(10, n)) })
  return (
    <div className="dl-slots">
      <span className="dl-slots-label" id="dl-slots-label">
        {t('dl.maxActive')}
      </span>
      <div className="dl-stepper" role="group" aria-labelledby="dl-slots-label">
        <button type="button" className="icon-btn" onClick={() => set(s.max_active - 1)} disabled={s.max_active <= 1 || save.isPending} aria-label={t('dl.fewer')}>
          −
        </button>
        <output aria-live="polite">{s.max_active}</output>
        <button type="button" className="icon-btn" onClick={() => set(s.max_active + 1)} disabled={s.max_active >= 10 || save.isPending} aria-label={t('dl.more')}>
          +
        </button>
      </div>
      <span className="dl-slots-state">{t('dl.slotsState', { a: running, b: waiting })}</span>
      {save.isError && <span className="field-error">{t('set.saveFailed')}</span>}
    </div>
  )
}

type QueueCtl = { pos: number; first: boolean; last: boolean; busy: boolean; up: () => void; down: () => void }

function DownloadCard({ x, onRemove, onPick, queue }: { x: Torrent; onRemove: () => void; onPick: () => void; queue?: QueueCtl }) {
  const { t } = useI18n()
  const device = useRole() === 'device'
  const qc = useQueryClient()
  const caps = useQuery({ queryKey: ['caps'], queryFn: api.caps, staleTime: 60_000, retry: false })
  const act = useMutation({
    mutationFn: (what: 'pause' | 'resume' | 'open') => (what === 'pause' ? torrentApi.pause(x.id) : what === 'resume' ? torrentApi.resume(x.id) : torrentApi.open(x.id)),
    onSettled: () => refreshTorrents(qc),
  })
  const videos = x.files.filter((f) => f.video)
  const selectedVideos = videos.filter((f) => f.selected)
  const firstVideo = selectedVideos[0] ?? videos[0]
  const done = x.state === 'completed'
  const paused = x.state === 'paused'
  const failed = x.state === 'error'
  const pct = Math.round(x.progress * 1000) / 10
  const eta = formatEta(x.eta_seconds)
  const facts = failed
    ? [x.total > 0 && t('dl.ofTotal', { a: formatBytes(x.downloaded), b: formatBytes(x.total) })]
    : done
    ? [formatBytes(x.total)]
    : [t('dl.ofTotal', { a: formatBytes(x.downloaded), b: formatBytes(x.total) }), eta && t('dl.eta', { eta })]
  return (
    <li className={`dl is-${x.state}`}>
      <span className="dl-icon" aria-hidden="true">
        <Icon name={done ? 'check' : x.media?.type === 'tv' ? 'tv' : 'film'} size={22} />
      </span>
      <div className="dl-body">
        <div className="dl-head">
          <h2 className="dl-title">{torrentTitle(x)}</h2>
          {x.mode === 'stream' && !done && <span className="tag tag-dim">{t('dl.fromStream')}</span>}
          {x.auto && <span className="tag tag-dim">{t('dl.auto')}</span>}
          {x.state === 'queued' && <span className="tag">{t('dl.queued')}</span>}
        </div>
        {x.name !== torrentTitle(x) && (
          <p className="dl-name" title={x.name}>
            {x.name}
          </p>
        )}
        <div
          className="dl-bar"
          role="progressbar"
          aria-label={t('dl.progress')}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={pct}
          aria-valuetext={`${pct} %`}
        >
          <span style={{ transform: `scaleX(${x.progress})` }} />
        </div>
        <p className="dl-meta">
          <span className={`dl-state${failed || x.state === 'vpn_blocked' ? ' is-bad' : ''}`}>
            {failed ? <Icon name="warn" size={14} /> : !done && !paused && <span className="bars" aria-hidden="true" />}
            {torrentLine(x)}
          </span>
          {!done && !failed && <span className="dl-pct">{pct.toLocaleString(undefined, { maximumFractionDigits: 1 })} %</span>}
          {facts.filter(Boolean).map((f) => (
            <span key={String(f)}>{f}</span>
          ))}
        </p>
        {failed && (
          <div className="dl-failed" role="alert">
            <p className="dl-error">{codeText(x.error_code)}</p>
            <div className="dl-failed-actions">
              <button type="button" className="btn btn-primary btn-s" onClick={() => act.mutate('resume')} disabled={act.isPending}>
                <Icon name="refresh" size={15} />
                {t('err.retry')}
              </button>
              <button type="button" className="btn btn-glass btn-s" onClick={onRemove}>
                <Icon name="trash" size={15} />
                {t('dl.removeShort')}
              </button>
            </div>
          </div>
        )}
        {x.state === 'checking' && x.downloaded > 0 && <p className="dl-resume">{t('dl.resuming', { size: formatBytes(x.downloaded) })}</p>}
      </div>
      <div className="dl-actions">
        {queue && (
          <span className="dl-move" role="group" aria-label={t('dl.queuePos', { n: queue.pos + 1 })}>
            <button type="button" className="icon-btn dl-up" onClick={queue.up} disabled={queue.first || queue.busy} aria-label={t('dl.moveUp', { title: torrentTitle(x) })} title={t('dl.moveUpShort')}>
              <Icon name="down" size={15} />
            </button>
            <button type="button" className="icon-btn" onClick={queue.down} disabled={queue.last || queue.busy} aria-label={t('dl.moveDown', { title: torrentTitle(x) })} title={t('dl.moveDownShort')}>
              <Icon name="down" size={15} />
            </button>
          </span>
        )}
        {firstVideo && (
          <Link to={torrentPlayPath(x.id, firstVideo.index)} className={`btn ${done ? 'btn-primary' : 'btn-glass'}`}>
            <Icon name="play" size={16} />
            {done ? t('common.play') : t('dl.playNow')}
          </Link>
        )}
        {!done &&
          !failed &&
          (paused ? (
            <button type="button" className="icon-btn" onClick={() => act.mutate('resume')} aria-label={t('dl.resume')} title={t('dl.resume')}>
              <Icon name="play" size={15} />
            </button>
          ) : (
            <button type="button" className="icon-btn" onClick={() => act.mutate('pause')} aria-label={t('dl.pause')} title={t('dl.pause')}>
              <Icon name="pause" size={15} />
            </button>
          ))}
        {videos.length > 1 && (
          <button type="button" className="icon-btn" onClick={onPick} aria-label={t('dl.pickFiles')} title={t('dl.pickFiles')}>
            <Icon name="layers" size={16} />
          </button>
        )}
        {x.folder && x.downloaded > 0 && !device && caps.data?.open_folder !== false && (
          <button type="button" className="icon-btn" onClick={() => act.mutate('open')} aria-label={t('dl.openFolder')} title={t('dl.openFolder')}>
            <Icon name="folder" size={16} />
          </button>
        )}
        <button type="button" className="icon-btn" onClick={onRemove} aria-label={t('dl.removeLabel', { title: torrentTitle(x) })} title={t('dl.remove')}>
          <Icon name="trash" size={16} />
        </button>
      </div>
    </li>
  )
}

function Modal({ labelledBy, onClose, children, wide }: { labelledBy: string; onClose: () => void; children: ReactNode; wide?: boolean }) {
  const ref = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const d = ref.current
    if (!d) return
    d.showModal()
    return () => d.close()
  }, [])
  return (
    <dialog
      ref={ref}
      className={`modal ${wide ? 'modal-files' : 'modal-confirm'}`}
      aria-labelledby={labelledBy}
      onClose={onClose}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      {children}
    </dialog>
  )
}

function RemoveDialog({ x, onClose }: { x: Torrent; onClose: () => void }) {
  const { t } = useI18n()
  // Deleting files from the PC's drive is done at the PC.
  const device = useRole() === 'device'
  const qc = useQueryClient()
  const remove = useMutation({
    mutationFn: (withFiles: boolean) => torrentApi.remove(x.id, withFiles),
    onSuccess: () => {
      refreshTorrents(qc)
      onClose()
    },
  })
  return (
    <Modal labelledBy="dl-remove-title" onClose={onClose}>
      <div className="confirm">
        <h2 id="dl-remove-title">{t('dl.removeTitle')}</h2>
        <p>{t('dl.removeText', { title: torrentTitle(x) })}</p>
        {remove.isError && (
          <p className="field-error" role="alert">
            {t('dl.removeFailed')}
          </p>
        )}
        <div className="actions">
          <button type="button" className="btn btn-glass" onClick={onClose} autoFocus>
            {t('common.cancel')}
          </button>
          <button type="button" className="btn btn-glass" onClick={() => remove.mutate(false)} disabled={remove.isPending}>
            {t('dl.removeKeep')}
          </button>
          {!device && (
            <button type="button" className="btn btn-danger" onClick={() => remove.mutate(true)} disabled={remove.isPending}>
              <Icon name="trash" size={17} />
              {t('dl.removeFiles')}
            </button>
          )}
        </div>
      </div>
    </Modal>
  )
}

/** Season packs: choose the episodes to load, named from TMDB where the file name gives SxxEyy. */
function FilePicker({ x, onClose }: { x: Torrent; onClose: () => void }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const videos = x.files.filter((f) => f.video)
  const [chosen, setChosen] = useState(() => new Set(videos.filter((f) => f.selected).map((f) => f.index)))
  const eps = videos.map((f) => episodeOfFile(f.path))
  const seasons = [...new Set(eps.filter((v) => v !== null).map((v) => v.s))]
  const tvId = x.media?.type === 'tv' ? x.media.id : undefined
  const names = useQuery({
    queryKey: ['season-names', tvId, seasons.join(',')],
    queryFn: async () => {
      const all = await Promise.all(seasons.map((n) => api.season(tvId ?? 0, n)))
      return new Map(all.flatMap((sd) => sd.episodes.map((ep) => [`${sd.season_number}-${ep.episode_number}`, ep.name] as const)))
    },
    enabled: tvId !== undefined && seasons.length > 0,
    staleTime: 10 * 60_000,
  })
  const save = useMutation({
    mutationFn: () => torrentApi.files(x.id, [...chosen].sort((a, z) => a - z)),
    onSuccess: () => {
      refreshTorrents(qc)
      onClose()
    },
  })
  const toggle = (i: number) =>
    setChosen((old) => {
      const next = new Set(old)
      if (next.has(i)) next.delete(i)
      else next.add(i)
      return next
    })
  const total = videos.filter((f) => chosen.has(f.index)).reduce((sum, f) => sum + f.size, 0)
  return (
    <Modal labelledBy="dl-files-title" onClose={onClose} wide>
      <div className="confirm files">
        <h2 id="dl-files-title">{t('dl.pickFiles')}</h2>
        <p>{t('dl.pickText')}</p>
        <div className="files-tools">
          <button type="button" className="chip" onClick={() => setChosen(new Set(videos.map((f) => f.index)))}>
            {t('dl.pickAll')}
          </button>
          <button type="button" className="chip" onClick={() => setChosen(new Set())}>
            {t('dl.pickNone')}
          </button>
          <span className="files-total">{t('dl.pickTotal', { n: chosen.size, size: formatBytes(total) })}</span>
        </div>
        <ul className="files-list">
          {videos.map((f, i) => {
            const ep = eps[i]
            const code = ep ? `S${String(ep.s).padStart(2, '0')}E${String(ep.e).padStart(2, '0')}` : null
            const name = ep ? names.data?.get(`${ep.s}-${ep.e}`) : undefined
            return (
              <li key={f.index}>
                <label className="file">
                  <input type="checkbox" checked={chosen.has(f.index)} onChange={() => toggle(f.index)} />
                  <span className="file-text">
                    <span className="file-title">
                      {code && <span className="card-ep">{code}</span>}
                      {name ?? f.name.split(/[\\/]/).pop()}
                    </span>
                    {name && <span className="file-path">{f.name.split(/[\\/]/).pop()}</span>}
                  </span>
                  <span className="file-size">{f.downloaded >= f.size && f.selected ? t('dl.fileDone') : formatBytes(f.size)}</span>
                </label>
              </li>
            )
          })}
        </ul>
        {save.isError && (
          <p className="field-error" role="alert">
            {t('dl.pickFailed')}
          </p>
        )}
        <div className="actions">
          <button type="button" className="btn btn-glass" onClick={onClose}>
            {t('common.cancel')}
          </button>
          <button type="button" className="btn btn-primary" onClick={() => save.mutate()} disabled={chosen.size === 0 || save.isPending}>
            <Icon name="download" size={17} />
            {t('dl.pickSave')}
          </button>
        </div>
      </div>
    </Modal>
  )
}

// ---------------------------------------------------------------- free search

type Category = 'all' | 'movie' | 'tv' | 'anime'
type SortBy = 'best' | 'seeders' | 'size' | 'quality'

function FreeSearch({ torrents }: { torrents: Torrent[] }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const navigate = useNavigate()
  const [params, setParams] = useSearchParams()
  const q = (params.get('q') ?? '').trim()
  const [draft, setDraft] = useState(q)
  const [cat, setCat] = useState<Category>('all')
  const [sort, setSort] = useState<SortBy>('best')
  const tf = useTorrentFilter()
  const tfKey = tf.params.toString()
  const anime = cat === 'anime'
  const res = useQuery({
    queryKey: ['torrent-search', q, anime, tfKey],
    queryFn: () => torrentApi.search(q, new URLSearchParams(tfKey), anime),
    enabled: q.length >= 2,
    staleTime: 5 * 60_000,
    retry: false,
    placeholderData: keepPreviousData,
  })
  const known = new Map(torrents.map((x) => [x.id, x]))
  const listed: ReadonlySet<string> = new Set(known.keys())
  const start = useMutation({
    mutationFn: ({ r, mode }: { r: TorrentResult; mode: 'stream' | 'download' }) =>
      torrentApi.start({ magnet: r.magnet, trackers: r.trackers, name: r.name, mode }),
    onSuccess: (x, v) => {
      refreshTorrents(qc)
      if (v.mode === 'stream') void navigate(torrentPlayPath(x.id))
    },
  })

  const shown = useMemo(() => {
    const isTv = (r: TorrentResult) => r.release.season !== null || r.release.pack
    const list = (res.data?.results ?? []).filter((r) => cat === 'all' || cat === 'anime' || (cat === 'tv') === isTv(r))
    const by: Record<SortBy, ((a: TorrentResult, z: TorrentResult) => number) | null> = {
      best: null,
      seeders: (a, z) => z.seeders - a.seeders,
      size: (a, z) => z.size - a.size,
      quality: (a, z) => (z.release.height ?? 0) - (a.release.height ?? 0) || z.seeders - a.seeders,
    }
    const cmp = by[sort]
    return cmp ? [...list].sort(cmp) : list
  }, [res.data, cat, sort])
  // Phone: sources, category, order and filters wait behind one "Filter" button, the hits start right below.
  const phone = useIsPhone()
  const [sheet, setSheet] = useState(false)
  const picked = activeCount(tf.filter) + Number(cat !== 'all') + Number(sort !== 'best')

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const v = draft.trim()
    setParams(v ? { tab: 'suche', q: v } : { tab: 'suche' }, { replace: true })
  }
  const CATS: { id: Category; label: Key }[] = [
    { id: 'all', label: 'ts.catAll' },
    { id: 'movie', label: 'ts.catMovies' },
    { id: 'tv', label: 'ts.catTv' },
    { id: 'anime', label: 'ts.catAnime' },
  ]
  const SORTS: { id: SortBy; label: Key }[] = [
    { id: 'best', label: 'ts.sortBest' },
    { id: 'seeders', label: 'ts.sortSources' },
    { id: 'quality', label: 'ts.sortQuality' },
    { id: 'size', label: 'ts.sortSize' },
  ]

  const tools = (
        <div className="ts-tools">
          <div className="seg ts-cats" role="radiogroup" aria-label={t('ts.category')}>
            {CATS.map((c) => (
              <button key={c.id} type="button" role="radio" aria-checked={cat === c.id} className="seg-item" onClick={() => setCat(c.id)}>
                {t(c.label)}
              </button>
            ))}
          </div>
          <label className="ts-sort">
            <span className="sr-only">{t('ts.sort')}</span>
            <span className="select">
              <select value={sort} onChange={(e) => setSort(e.target.value as SortBy)} aria-label={t('ts.sort')}>
                {SORTS.map((s) => (
                  <option key={s.id} value={s.id}>
                    {t(s.label)}
                  </option>
                ))}
              </select>
              <Icon name="down" size={14} />
            </span>
          </label>
        </div>
  )

  return (
    <div className="ts">
      <form className="ts-form" role="search" onSubmit={submit}>
        <span className="ts-field">
          <Icon name="search" size={18} />
          <input
            type="search"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder={t('ts.placeholder')}
            aria-label={t('ts.placeholder')}
            autoComplete="off"
            spellCheck={false}
            enterKeyHint="search"
          />
        </span>
        <button type="submit" className="btn btn-primary" disabled={draft.trim().length < 2}>
          {t('ts.go')}
        </button>
      </form>
      {!phone && <p className="field-hint ts-hint">{t('ts.hint')}</p>}

      {q.length >= 2 && !phone && tools}

      {q.length < 2 ? (
        <EmptyState icon="search" title={t('ts.emptyTitle')} text={t('ts.emptyText')} />
      ) : res.error ? (
        <ErrorState error={res.error} onRetry={() => void res.refetch()} />
      ) : res.isPending ? (
        <div className="ts-wait" role="status">
          <span className="bars" aria-hidden="true" />
          {t('ts.searching')}
        </div>
      ) : (
        <>
          {phone ? (
            <div className="ts-bar">
              <p className="ts-count" role="status">
                {res.isPlaceholderData && <span className="bars" aria-hidden="true" />}
                {t('tor.count', { n: shown.length })}
              </p>
              <button
                type="button"
                className={`tf-toggle${picked > 0 ? ' has-active' : ''}`}
                aria-haspopup="dialog"
                aria-expanded={sheet}
                onClick={() => setSheet(true)}
              >
                <Icon name="sliders" size={15} />
                <span className="tf-toggle-text">{t('tf.title')}</span>
                {picked > 0 && <span className="tf-badge">{picked}</span>}
              </button>
              <Sheet open={sheet} onClose={() => setSheet(false)} title={t('tf.title')} className="tf-sheet ts-sheet">
                {tools}
                {res.data.total > 0 && (
                  <FilterBar bare data={res.data} busy={res.isPlaceholderData} filter={tf.filter} mode={tf.mode} onFilter={tf.setFilter} onMode={tf.setMode} />
                )}
                <ul className="ts-sources" aria-label={t('ts.sources')}>
                  {res.data.sources
                    .filter((s) => s.status !== 'skipped' && s.status !== 'off')
                    .map((s) => (
                      <li key={s.id} className={`ts-source is-${s.status}`} title={`${s.ms} ms`}>
                        {sourceName(s.id)}
                        <span>{s.status === 'ok' ? s.count : s.status === 'blocked' ? t('ts.blocked') : t('ts.failed')}</span>
                      </li>
                    ))}
                </ul>
                <div className="sheet-actions">
                  <button
                    type="button"
                    className="btn btn-glass"
                    disabled={picked === 0}
                    onClick={() => (tf.setFilter(EMPTY_FILTER), setCat('all'), setSort('best'))}
                  >
                    {t('tf.reset')}
                  </button>
                  <button type="button" className="btn btn-primary" onClick={() => setSheet(false)}>
                    {t('tor.count', { n: shown.length })}
                  </button>
                </div>
              </Sheet>
            </div>
          ) : (
            <>
          <ul className="ts-sources" aria-label={t('ts.sources')}>
            {res.data.sources
              .filter((s) => s.status !== 'skipped' && s.status !== 'off')
              .map((s) => (
                <li key={s.id} className={`ts-source is-${s.status}`} title={`${s.ms} ms`}>
                  {sourceName(s.id)}
                  <span>{s.status === 'ok' ? s.count : s.status === 'blocked' ? t('ts.blocked') : t('ts.failed')}</span>
                </li>
              ))}
          </ul>
          {res.data.total > 0 && (
            <FilterBar data={res.data} busy={res.isPlaceholderData} filter={tf.filter} mode={tf.mode} onFilter={tf.setFilter} onMode={tf.setMode} />
          )}
            </>
          )}
          {start.error instanceof ApiError && start.error.code === 'needs_vpn' ? (
            <NeedsVpn />
          ) : (
            start.isError && (
              <p className="field-error" role="alert">
                {start.error instanceof ApiError ? errorText(start.error) : t('tor.downloadFailed')}
              </p>
            )
          )}
          {shown.length === 0 ? (
            res.data.total > 0 && res.data.matched === 0 ? (
              <FilterEmpty data={res.data} filter={tf.filter} onFilter={tf.setFilter} />
            ) : (
              <EmptyState icon="search" title={t('search.noneTitle')} text={t('ts.none', { q })} />
            )
          ) : (
            <>
              {!phone && <p className="ts-count">{t('tor.count', { n: shown.length })}</p>}
              <ol className="ts-list">
                {shown.map((r) => (
                  <TorrentRow
                    key={r.infohash}
                    r={r}
                    known={known.get(r.infohash)}
                    inList={listed.has(r.infohash) || (start.isPending && start.variables?.r.infohash === r.infohash)}
                    onPlay={() => start.mutate({ r, mode: 'stream' })}
                    onDownload={() => start.mutate({ r, mode: 'download' })}
                  />
                ))}
              </ol>
            </>
          )}
        </>
      )}
    </div>
  )
}

// ---------------------------------------------------------------- settings


function SettingsForm({ vpn }: { vpn: Vpn | undefined }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const saved = useQuery({ queryKey: ['torrent-settings'], queryFn: torrentApi.settings, retry: false })
  const [draft, setDraft] = useState<TorrentSettings | null>(null)
  const form = draft ?? saved.data
  const save = useMutation({
    mutationFn: torrentApi.settingsSave,
    onSuccess: (_v, next) => {
      qc.setQueryData(['torrent-settings'], next)
      setDraft(null)
      refreshTorrents(qc)
    },
  })
  if (saved.error) return <ErrorState error={saved.error} onRetry={() => void saved.refetch()} />
  if (!form) return <PageLoader />
  const set = (patch: Partial<TorrentSettings>) => {
    save.reset()
    setDraft({ ...form, ...patch })
  }
  const dirty = draft !== null
  const up = vpn?.state === 'up'

  return (
    <form
      className="dl-settings"
      onSubmit={(e) => {
        e.preventDefault()
        if (draft) save.mutate(draft)
      }}
    >
      <section className="set-card" aria-labelledby="set-vpn">
        <h2 className="set-title" id="set-vpn">
          <Icon name="shield" size={18} />
          {t('set.vpnTitle')}
        </h2>
        <p className="set-status">
          <VpnBadge vpn={vpn} always />
          <span>{up ? (vpn.country ? t('vpn.upIn', { country: vpn.country }) : t('vpn.up')) : vpn?.state === 'connecting' ? t('vpn.connecting') : t('vpn.downHint')}</span>
        </p>
        <VpnNotice vpn={vpn} noLink />
        <div className="field">
          <span className="field-label">{t('set.vpnFile')}</span>
          <span className="field-hint">{t('set.vpnFileHint')}</span>
          {/* The same import as in the first-run setup: the file is checked and stored by the server. */}
          <VpnImport has={!!vpn?.config_found} onChange={() => refreshTorrents(qc)} />
          <DirectSwitch />
          <Link to="/einstellungen#torrents" className="vpn-notice-link">
            {t('set.moreInSettings')}
          </Link>
        </div>
      </section>

      <section className="set-card" aria-labelledby="set-dl">
        <h2 className="set-title" id="set-dl">
          <Icon name="folder" size={18} />
          {t('set.dlTitle')}
        </h2>
        <label className="field">
          <span className="field-label">{t('set.moviesDir')}</span>
          <input className="input" value={form.movies_dir} onChange={(e) => set({ movies_dir: e.target.value })} spellCheck={false} />
        </label>
        <label className="field">
          <span className="field-label">{t('set.seriesDir')}</span>
          <input className="input" value={form.series_dir} onChange={(e) => set({ series_dir: e.target.value })} spellCheck={false} />
        </label>
        <label className="field set-upload">
          <span className="field-label">{t('set.upload')}</span>
          <span className="set-unit">
            <input
              className="input"
              type="number"
              min={1}
              max={1000}
              value={form.upload_kbps}
              onChange={(e) => set({ upload_kbps: Math.max(1, Math.min(1000, Math.round(Number(e.target.value) || 1))) })}
            />
            <span>KB/s</span>
          </span>
          <span className="field-hint">{t('set.uploadHint')}</span>
        </label>
        <QueueFields form={form} set={set} />
      </section>

      <div className="set-actions">
        {save.isError && (
          <p className="field-error" role="alert">
            {save.error instanceof ApiError ? errorText(save.error) : t('set.saveFailed')}
          </p>
        )}
        {save.isSuccess && !dirty && (
          <p className="set-saved" role="status">
            <Icon name="check" size={15} stroke={2.2} />
            {t('set.saved')}
          </p>
        )}
        <button type="button" className="btn btn-glass" disabled={!dirty} onClick={() => (setDraft(null), save.reset())}>
          {t('set.reset')}
        </button>
        <button type="submit" className="btn btn-primary" disabled={!dirty || save.isPending}>
          {t('set.save')}
        </button>
      </div>
    </form>
  )
}
