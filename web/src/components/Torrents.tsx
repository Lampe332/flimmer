import { useRole } from '../lib/session'
import { useId, useState, type ReactNode } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router'
import { useI18n } from '../i18n'
import type { Filter } from '../lib/api'
import { formatHeight } from '../lib/format'
import {
  audioChips,
  formatBytes,
  likelyDirect,
  refreshTorrents,
  sourceName,
  torrentApi,
  torrentFit,
  useTorrents,
  vpnProblem,
  type Torrent,
  type TorrentResult,
  type Vpn,
} from '../lib/torrents'
import Icon from './Icon'
import { valueLabel } from './TorrentFilters'

type RowProps = {
  r: TorrentResult
  filter?: Filter
  /** This torrent plays right now. */
  active?: boolean
  /** The automatic choice would take this one. */
  best?: boolean
  /** Already in the download list. */
  inList?: boolean
  /** The same torrent in the download list (its files are known then). */
  known?: Torrent
  onPlay?: () => void
  onDownload?: () => void
}

/** Release name without the advertising prefix many uploads carry ("www.UIndex.org - ", "[ www.site.com ] "). */
function cleanName(name: string): string {
  const cleaned = name
    .replace(/^\s*\[\s*(www[\s.])?[\w-]+([\s.][\w-]+)*[\s.](org|com|net|to|cc|me|info|io|tv|ws|nl)\s*\]\s*[-–]?\s*/i, '')
    .replace(/^\s*(www[\s.])?[\w-]+[\s.](org|com|net|to|cc|me|info|io|tv|ws|nl)\s+[-–]\s+/i, '')
  return cleaned.trim() || name
}

/** One torrent: resolution, size, seeders, then small chips for language, source, HDR, codec; details fold out. */
export function TorrentRow({ r, filter, active, best, inList, known, onPlay, onDownload }: RowProps) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const detailsId = useId()
  const fit = filter ? torrentFit(r, filter) : { height: true, lang: true }
  const off = !(fit.height && fit.lang)
  const rel = r.release
  const chips = audioChips(rel)
  const quality = formatHeight(rel.height) ?? t('scan.unknownQuality')
  const facts = (
    <>
      <span className="trow-top">
        <span className={`fact fact-quality${fit.height ? '' : ' is-miss'}`}>{quality}</span>
        <span className="trow-num">{formatBytes(r.size)}</span>
        {r.seeders_unknown && r.seeders === 0 ? (
          <span className="trow-num trow-seeds is-low" title={t('td.seedsUnknownTitle')}>
            <Icon name="seed" size={13} stroke={2.2} />?
          </span>
        ) : (
          <span className={`trow-num trow-seeds${r.seeders < 5 ? ' is-low' : ''}`} title={t('tor.seedersTitle', { n: r.seeders })}>
            <Icon name="seed" size={13} stroke={2.2} />
            {r.seeders.toLocaleString()}
          </span>
        )}
        <span className="trow-tag">
          {active ? (
            <span className="tag tag-live">
              <span className="bars" aria-hidden="true" />
              {t('scan.live')}
            </span>
          ) : best ? (
            <span className="tag">{t('scan.best')}</span>
          ) : off ? (
            <span className="tag tag-dim">{t('scan.offFilter')}</span>
          ) : null}
        </span>
      </span>
      <span className="trow-name" title={r.name}>
        {cleanName(r.name)}
      </span>
      <span className="trow-chips">
        {chips.map((c) => (
          <span key={c.label} className={`chip-s${c.strong ? ' is-lang' : ''}${fit.lang ? '' : ' is-miss'}`}>
            {c.label}
          </span>
        ))}
        {rel.source && <span className={`chip-s${rel.cam ? ' is-warn' : ''}`}>{rel.source}</span>}
        {rel.hdr && <span className="chip-s is-hdr">{rel.hdr}</span>}
        {rel.codec && <span className="chip-s">{rel.codec}</span>}
        {rel.pack && <span className="chip-s">{t('tor.pack')}</span>}
        {likelyDirect(r) && (
          <span className="chip-s is-direct" title={t('tor.directTitle')}>
            {t('tor.direct')}
          </span>
        )}
        <span className="trow-sources">{r.sources.map(sourceName).join(' · ')}</span>
      </span>
    </>
  )
  return (
    <li className={`trow${active ? ' is-active' : ''}${off ? ' is-off' : ''}${open ? ' is-open' : ''}`}>
      <div className="trow-line">
      {onPlay ? (
        <button
          type="button"
          className="trow-main"
          aria-pressed={active}
          aria-label={t('tor.playLabel', { name: r.name })}
          onClick={onPlay}
        >
          <span className="trow-info">{facts}</span>
          <span className="trow-play" aria-hidden="true">
            <Icon name={active ? 'bolt' : 'play'} size={14} />
          </span>
        </button>
      ) : (
        <div className="trow-main">
          <span className="trow-info">{facts}</span>
        </div>
      )}
      {onDownload && (
        <button
          type="button"
          className={`icon-btn trow-dl${inList ? ' is-done' : ''}`}
          onClick={onDownload}
          disabled={inList}
          aria-label={inList ? t('tor.inList') : t('tor.downloadLabel', { name: r.name })}
          title={inList ? t('tor.inList') : t('tor.download')}
        >
          <Icon name={inList ? 'check' : 'download'} size={17} />
        </button>
      )}
      <button
        type="button"
        className="icon-btn trow-more"
        aria-expanded={open}
        aria-controls={detailsId}
        aria-label={t('td.toggle', { name: r.name })}
        title={open ? t('td.hide') : t('td.show')}
        onClick={() => setOpen((o) => !o)}
      >
        <Icon name="info" size={16} />
      </button>
      </div>
      {open && <TorrentDetails id={detailsId} r={r} known={known} />}
    </li>
  )
}

/** Everything the release name and the sources tell, plus the files once the torrent is known. */
function TorrentDetails({ id, r, known }: { id: string; r: TorrentResult; known?: Torrent }) {
  const { t } = useI18n()
  const rel = r.release
  const yes = (v: boolean, label: string) => (v ? label : null)
  const flags = [
    yes(rel.dual, 'Dual'),
    rel.dub && t('td.dub', { kind: rel.dub }),
    yes(rel.proper, 'PROPER'),
    yes(rel.repack, 'REPACK'),
    yes(rel['3d'], '3D'),
    rel.edition,
    yes(rel.cam, 'CAM'),
  ].filter(Boolean)
  const episode =
    rel.season !== null
      ? rel.episode !== null
        ? `S${String(rel.season).padStart(2, '0')}E${String(rel.episode).padStart(2, '0')}`
        : t('tor.seasonN', { n: rel.season })
      : null
  const rows: [string, ReactNode][] = [
    [t('tf.quality'), formatHeight(rel.height) ?? t('tf.unknown')],
    [t('tf.source'), rel.source ?? t('tf.unknown')],
    [t('tf.codec'), [rel.codec, rel.bit_depth && `${rel.bit_depth} Bit`].filter(Boolean).join(' · ') || t('tf.unknown')],
    [t('tf.hdr'), rel.hdr_all.length > 0 ? rel.hdr_all.join(' · ') : 'SDR'],
    [t('tf.audio'), [rel.audio, rel.channels].filter(Boolean).join(' · ') || t('tf.unknown')],
    [t('tf.language'), rel.languages.length > 0 ? rel.languages.map((l) => valueLabel('language', l, t)).join(', ') : t('tf.langNone')],
    [t('tf.subs'), rel.subtitles.length > 0 ? rel.subtitles.join(', ') : '–'],
    [t('tf.group'), rel.group ?? '–'],
  ]
  if (episode) rows.push([t('td.episode'), rel.pack ? `${episode} (${t('tor.pack')})` : episode])
  if (rel.year) rows.push([t('td.year'), String(rel.year)])
  if (flags.length > 0) rows.push([t('td.flags'), flags.join(' · ')])
  rows.push(
    [t('td.size'), formatBytes(r.size)],
    [
      t('tf.seeders'),
      r.seeders_unknown && r.seeders === 0
        ? t('td.seedsUnknown')
        : r.sources.length > 1
          ? t('td.seedsSum', { n: r.seeders, max: r.seeders_max })
          : r.seeders.toLocaleString(),
    ],
    [t('td.leechers'), r.leechers.toLocaleString()],
  )
  if (r.age_days !== null) rows.push([t('td.age'), t('td.ageDays', { n: r.age_days })])
  rows.push(
    [t('td.sources'), r.sources.map(sourceName).join(', ')],
    [t('td.trackers'), String(r.trackers.length)],
    [t('td.hash'), <code key="h">{r.infohash}</code>],
  )
  const files = known?.files ?? []
  return (
    <div className="tdetails" id={id}>
      <p className="tdetails-name">{r.name}</p>
      <dl className="tdetails-grid">
        {rows.map(([k, v]) => (
          <div key={k}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
      <p className="tdetails-sub">{t('td.files')}</p>
      {files.length > 0 ? (
        <ul className="tdetails-files">
          {files.map((f) => (
            <li key={f.index} className={f.index === r.file_index ? 'is-wanted' : undefined}>
              <span className="tdetails-file" title={f.name}>
                {f.name}
              </span>
              <span>{formatBytes(f.size)}</span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="field-hint">{r.file_index !== null ? t('td.filesLaterIndex', { n: r.file_index + 1 }) : t('td.filesLater')}</p>
      )}
    </div>
  )
}

/** Small shield while the tunnel is up: "VPN · NL". Nothing otherwise, unless `always`. */
export function VpnBadge({ vpn, always }: { vpn: Vpn | undefined; always?: boolean }) {
  const { t } = useI18n()
  if (!vpn) return null
  // Without VPN (allowed by the user): always visible, in the warning colour, whether or not a torrent runs.
  if (vpn.state !== 'blocked' && (vpn.direct || vpn.route === 'direct')) {
    const where = vpn.state === 'up' && vpn.country ? t('direct.badgeIn', { country: vpn.country }) : t('direct.badgeTitle')
    return (
      <span className="vpn-badge is-direct" title={where} role="status">
        <Icon name="warn" size={15} stroke={2} />
        <span>{t('direct.badge')}</span>
        <span className="sr-only">{where}</span>
      </span>
    )
  }
  if (vpn.state === 'up') {
    const where = vpn.country ? t('vpn.upIn', { country: vpn.country }) : t('vpn.up')
    return (
      <span className="vpn-badge is-up" title={where} role="status">
        <Icon name="shield" size={15} stroke={2} />
        <span>{vpn.country ? `VPN · ${vpn.country}` : 'VPN'}</span>
        <span className="sr-only">{where}</span>
      </span>
    )
  }
  if (vpn.state === 'connecting')
    return (
      <span className="vpn-badge is-connecting" role="status">
        <span className="bars" aria-hidden="true" />
        {t('vpn.connecting')}
      </span>
    )
  if (!always) return null
  const problem = vpnProblem(vpn)
  return (
    <span className={`vpn-badge${problem ? ' is-problem' : ''}`} title={problem ? t(problem.title) : t('vpn.downHint')}>
      <Icon name="shield" size={15} stroke={2} />
      {problem ? t('vpn.problemShort') : t('vpn.down')}
    </span>
  )
}

/** The header shield, fed by the shared torrent poll. */
export function HeaderVpn() {
  const q = useTorrents()
  return <VpnBadge vpn={q.data?.vpn} />
}

/** Friendly box when the tunnel cannot work: config missing, kill switch, Proton app blocking the handshake. */
export function VpnNotice({ vpn, compact, noLink }: { vpn: Vpn | undefined; compact?: boolean; noLink?: boolean }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const release = useMutation({ mutationFn: torrentApi.vpnRelease, onSuccess: () => refreshTorrents(qc) })
  const device = useRole() === 'device'
  const problem = vpnProblem(vpn)
  if (!problem) return null
  return (
    <div className={`vpn-notice${compact ? ' is-compact' : ''}`} role="alert">
      <Icon name="shield" size={18} />
      <div>
        <p className="vpn-notice-title">{t(problem.title)}</p>
        <p>{t(problem.text)}</p>
        {vpn?.state === 'blocked' ? (
          <button type="button" className="btn btn-glass vpn-notice-btn" onClick={() => release.mutate()} disabled={release.isPending || device}>
            {t('vpn.release')}
          </button>
        ) : (
          problem.title === 'vpn.missingTitle' &&
          !noLink && (
            <Link to="/downloads?tab=einstellungen" className="vpn-notice-link">
              {t('vpn.toSettings')}
            </Link>
          )
        )}
      </div>
    </div>
  )
}
