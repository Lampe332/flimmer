import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useI18n } from '../i18n'
import { refreshTorrents, sourceName, torrentApi, type SourceInfo, type TorrentSettings } from '../lib/torrents'
import { Segmented, Toggle } from './Controls'
import Icon from './Icon'

/** Queue size and the space a download leaves free (Downloads tab and settings page). */
export function QueueFields({ form, set }: { form: TorrentSettings; set: (p: Partial<TorrentSettings>) => void }) {
  const { t } = useI18n()
  const clamp = (v: string, lo: number, hi: number) => Math.max(lo, Math.min(hi, Math.round(Number(v) || lo)))
  return (
    <>
      <label className="field set-upload">
        <span className="field-label">{t('dl.maxActive')}</span>
        <span className="set-unit">
          <input className="input" type="number" min={1} max={10} value={form.max_active} onChange={(e) => set({ max_active: clamp(e.target.value, 1, 10) })} />
        </span>
        <span className="field-hint">{t('set.maxActiveHint')}</span>
      </label>
      <label className="field set-upload">
        <span className="field-label">{t('set.minFree')}</span>
        <span className="set-unit">
          <input className="input" type="number" min={0} max={2000} value={form.min_free_gb} onChange={(e) => set({ min_free_gb: clamp(e.target.value, 0, 2000) })} />
          <span>GB</span>
        </span>
        <span className="field-hint">{t('set.minFreeHint')}</span>
      </label>
    </>
  )
}

/** Saves only the "without VPN" fields on top of the server's current settings. */
async function saveDirect(patch: Pick<TorrentSettings, 'allow_direct' | 'direct'>): Promise<TorrentSettings> {
  const current = await torrentApi.settings()
  const next = { ...current, ...patch }
  await torrentApi.settingsPut(next)
  return next
}

/** The one-time warning before torrents may run without VPN; confirm or cancel. */
function DirectWarning({ onYes, onNo, busy }: { onYes: () => void; onNo: () => void; busy: boolean }) {
  const { t } = useI18n()
  return (
    <div className="direct-warn" role="alertdialog" aria-labelledby="direct-warn-h" aria-describedby="direct-warn-t">
      <p className="direct-warn-title" id="direct-warn-h">
        <Icon name="warn" size={18} />
        {t('direct.warnTitle')}
      </p>
      <ul id="direct-warn-t">
        <li>{t('direct.warnUpload')}</li>
        <li>{t('direct.warnIp')}</li>
        <li>{t('direct.warnLaw')}</li>
        <li>{t('direct.warnYours')}</li>
      </ul>
      <div className="direct-warn-actions">
        <button type="button" className="btn btn-glass" onClick={onNo} autoFocus>
          {t('common.cancel')}
        </button>
        <button type="button" className="btn btn-danger" onClick={onYes} disabled={busy}>
          {t('direct.confirm')}
        </button>
      </div>
    </div>
  )
}

/**
 * "Torrents ohne VPN erlauben" (off by default) and, once allowed, the route: through the VPN or directly.
 * With `compact` (first-run setup) it is a single "Ohne VPN nutzen" button with the same warning.
 */
export function DirectSwitch({ compact, onChange }: { compact?: boolean; onChange?: () => void }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const saved = useQuery({ queryKey: ['torrent-settings'], queryFn: torrentApi.settings, retry: false })
  const [asking, setAsking] = useState(false)
  const save = useMutation({
    mutationFn: saveDirect,
    onSuccess: (next) => {
      qc.setQueryData(['torrent-settings'], next)
      setAsking(false)
      refreshTorrents(qc)
      onChange?.()
    },
  })
  const s = saved.data
  if (!s) return null
  const warning = asking && <DirectWarning busy={save.isPending} onNo={() => setAsking(false)} onYes={() => save.mutate({ allow_direct: true, direct: s.direct })} />
  if (compact) {
    return s.allow_direct ? (
      <p className="direct-on">
        <Icon name="warn" size={15} />
        {t('direct.onShort')}
      </p>
    ) : (
      <div className="direct-compact">
        {!asking && (
          <button type="button" className="btn btn-glass" onClick={() => setAsking(true)}>
            {t('direct.useWithout')}
          </button>
        )}
        {warning}
      </div>
    )
  }
  return (
    <div className="direct">
      <Toggle
        label={t('direct.allow')}
        hint={t('direct.allowHint')}
        checked={s.allow_direct}
        disabled={save.isPending}
        onChange={(on) => (on ? setAsking(true) : save.mutate({ allow_direct: false, direct: false }))}
      />
      {warning}
      {s.allow_direct && (
        <div className="ar-field">
          <span className="field-label">{t('direct.route')}</span>
          <Segmented<'vpn' | 'direct'>
            label={t('direct.route')}
            value={s.direct ? 'direct' : 'vpn'}
            options={[
              { id: 'vpn', label: t('direct.viaVpn') },
              { id: 'direct', label: t('direct.viaDirect') },
            ]}
            onChange={(r) => save.mutate({ allow_direct: true, direct: r === 'direct' })}
          />
          <span className="field-hint">{t('direct.routeHint')}</span>
        </div>
      )}
      {save.isError && <p className="field-error">{t('set.saveFailed')}</p>}
    </div>
  )
}

/** Every torrent search source with an on/off switch; saved at once (the server forgets cached searches then). */
export function SourceList() {
  const { t } = useI18n()
  const qc = useQueryClient()
  const sources = useQuery({ queryKey: ['torrent-sources'], queryFn: torrentApi.sources, retry: false })
  const save = useMutation({
    mutationFn: async ({ id, on }: { id: string; on: boolean }) => {
      const s = await torrentApi.settings()
      const off = new Set(s.sources_off)
      if (on) off.delete(id)
      else off.add(id)
      const next = { ...s, sources_off: [...off] }
      await torrentApi.settingsPut(next)
      return next
    },
    onSuccess: (next) => {
      qc.setQueryData(['torrent-settings'], next)
      qc.setQueryData<SourceInfo[]>(['torrent-sources'], (old) => old?.map((x) => ({ ...x, on: !next.sources_off.includes(x.id) })))
      void qc.invalidateQueries({ queryKey: ['torrents-for'] })
      void qc.invalidateQueries({ queryKey: ['torrent-search'] })
    },
  })
  const list = sources.data ?? []
  const kind = (s: SourceInfo) =>
    [s.anime && t('src.anime'), s.imdb && t('src.imdb'), !s.seeders && t('src.noSeeds'), !s.default_on && t('src.offByDefault')].filter(Boolean).join(' · ')
  const on = list.filter((s) => s.on).length
  return (
    <div className="src-list">
      <p className="pref-label">{t('src.title', { a: on, b: list.length })}</p>
      <p className="field-hint">{t('src.hint')}</p>
      <div className="src-grid">
        {list.map((s) => (
          <Toggle key={s.id} label={sourceName(s.id)} hint={kind(s) || undefined} checked={s.on} disabled={save.isPending} onChange={(v) => save.mutate({ id: s.id, on: v })} />
        ))}
      </div>
      {(save.isError || sources.isError) && <p className="field-error">{t('set.saveFailed')}</p>}
    </div>
  )
}
