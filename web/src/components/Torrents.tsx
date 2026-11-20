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
  type TorrentResult,
  type Vpn,
} from '../lib/torrents'
import Icon from './Icon'

type RowProps = {
  r: TorrentResult
  filter?: Filter
  /** This torrent plays right now. */
  active?: boolean
  /** The automatic choice would take this one. */
  best?: boolean
  /** Already in the download list. */
  inList?: boolean
  onPlay?: () => void
  onDownload?: () => void
}

/** One torrent: resolution, size, seeders, then small chips for language, source, HDR, codec. */
export function TorrentRow({ r, filter, active, best, inList, onPlay, onDownload }: RowProps) {
  const { t } = useI18n()
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
        <span className={`trow-num trow-seeds${r.seeders < 5 ? ' is-low' : ''}`} title={t('tor.seedersTitle', { n: r.seeders })}>
          <Icon name="seed" size={13} stroke={2.2} />
          {r.seeders.toLocaleString()}
        </span>
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
        {r.name}
      </span>
      <span className="trow-chips">
        {chips.map((c) => (
          <span key={c.label} className={`chip-s${c.strong ? ' is-lang' : ''}${fit.lang ? '' : ' is-miss'}`}>
            {c.label}
          </span>
        ))}
        {rel.source && <span className={`chip-s${rel.source === 'CAM' ? ' is-warn' : ''}`}>{rel.source}</span>}
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
    <li className={`trow${active ? ' is-active' : ''}${off ? ' is-off' : ''}`}>
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
    </li>
  )
}

/** Small shield while the tunnel is up: "VPN · NL". Nothing otherwise, unless `always`. */
export function VpnBadge({ vpn, always }: { vpn: Vpn | undefined; always?: boolean }) {
  const { t } = useI18n()
  if (!vpn) return null
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
  const problem = vpnProblem(vpn)
  if (!problem) return null
  return (
    <div className={`vpn-notice${compact ? ' is-compact' : ''}`} role="alert">
      <Icon name="shield" size={18} />
      <div>
        <p className="vpn-notice-title">{t(problem.title)}</p>
        <p>{t(problem.text)}</p>
        {vpn?.state === 'blocked' ? (
          <button type="button" className="btn btn-glass vpn-notice-btn" onClick={() => release.mutate()} disabled={release.isPending}>
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
