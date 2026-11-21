import { useEffect, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useI18n } from '../i18n'
import { formatClock, locale } from '../lib/format'
import { lanApi, sessionApi, type LanDevice, type LanState } from '../lib/session'
import { PrefRow, Toggle } from './Controls'
import Icon from './Icon'

function when(iso: string): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return ''
  return new Intl.DateTimeFormat(locale(), { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }).format(d)
}

function Device({ d, onRemove, busy }: { d: LanDevice; onRemove: () => void; busy: boolean }) {
  const { t } = useI18n()
  const [ask, setAsk] = useState(false)
  return (
    <li className="lan-device">
      <span className="lan-device-text">
        <span className="pref-label">{d.name}</span>
        <span className="field-hint">{t('lan.deviceSeen', { when: when(d.last_seen), ip: d.last_ip })}</span>
      </span>
      {!ask ? (
        <button type="button" className="btn btn-glass btn-s" onClick={() => setAsk(true)}>
          <Icon name="trash" size={15} />
          {t('lan.deviceRemove')}
        </button>
      ) : (
        <span className="pref-confirm-row" role="group" aria-label={t('lan.deviceRemoveAsk')}>
          <span className="pref-confirm">{t('lan.deviceRemoveAsk')}</span>
          <button type="button" className="btn btn-danger btn-s" disabled={busy} autoFocus onClick={onRemove}>
            {t('lan.deviceRemoveYes')}
          </button>
          <button type="button" className="btn btn-glass btn-s" onClick={() => setAsk(false)}>
            {t('common.cancel')}
          </button>
        </span>
      )}
    </li>
  )
}

/** PIN and QR code of an open pairing. The server counts the time; the page only shows it. */
function PairingBox({ lan, onClose }: { lan: LanState; onClose: () => void }) {
  const { t } = useI18n()
  const p = lan.pairing
  const [left, setLeft] = useState(p?.expires_in ?? 0)
  useEffect(() => {
    setLeft(p?.expires_in ?? 0)
    const timer = window.setInterval(() => setLeft((n) => Math.max(0, n - 1)), 1000)
    return () => window.clearInterval(timer)
  }, [p?.pin, p?.expires_in])
  if (!p) return null
  return (
    <div className="lan-pairing" role="group" aria-label={t('lan.pairStart')}>
      {p.qr && <img className="lan-qr" src={`data:image/svg+xml;utf8,${encodeURIComponent(p.qr)}`} alt={t('lan.pairQr')} width={200} height={200} />}
      <div className="lan-pairing-text">
        <span className="field-label">{t('lan.pairPin')}</span>
        <output className="lan-pin" aria-live="polite">
          {p.pin.slice(0, 3)} {p.pin.slice(3)}
        </output>
        <p className="field-hint">{t('lan.pairHelp')}</p>
        <p className="field-hint">{t('lan.pairExpires', { time: formatClock(left) })}</p>
        <button type="button" className="btn btn-glass btn-s" onClick={onClose}>
          {t('lan.pairClose')}
        </button>
      </div>
    </div>
  )
}

/** "Handy & Tablet im WLAN" in the PC's settings: the switch, the address, pairing and the device list. */
export function LanSettings() {
  const { t } = useI18n()
  const qc = useQueryClient()
  const [added, setAdded] = useState<string | null>(null)
  const known = useRef<Set<string> | null>(null)
  const lan = useQuery({
    queryKey: ['lan'],
    queryFn: lanApi.get,
    retry: false,
    // While a pairing is open or the address is still being opened, look again soon.
    refetchInterval: (q) => (q.state.data?.pairing || (q.state.data?.enabled && !q.state.data.active_ip) ? 2000 : 15000),
  })
  const refresh = (next?: LanState) => (next ? qc.setQueryData(['lan'], next) : void qc.invalidateQueries({ queryKey: ['lan'] }))
  const turn = useMutation({ mutationFn: (v: { enabled: boolean; ip: string | null }) => lanApi.put(v.enabled, v.ip), onSettled: () => refresh() })
  const pairing = useMutation({ mutationFn: lanApi.pairingStart, onSuccess: (next) => refresh(next) })
  const close = useMutation({ mutationFn: lanApi.pairingStop, onSettled: () => refresh() })
  const remove = useMutation({ mutationFn: lanApi.deviceRemove, onSettled: () => refresh() })

  const d = lan.data
  // A device that appears while the page is open was just paired: say so.
  useEffect(() => {
    if (!d) return
    const ids = new Set(d.devices.map((x) => x.id))
    const fresh = known.current && d.devices.find((x) => !known.current?.has(x.id))
    known.current = ids
    if (fresh) setAdded(fresh.name)
  }, [d])

  if (!d) return lan.isError ? <p className="field-error">{t('pref.failed')}</p> : <p className="field-hint">{t('common.loading')}</p>
  const failed = turn.isError || pairing.isError || close.isError || remove.isError
  return (
    <>
      <p className="field-hint">{t('lan.intro')}</p>
      <Toggle
        label={t('lan.enable')}
        hint={t('lan.enableHint')}
        checked={d.enabled}
        disabled={turn.isPending}
        onChange={(enabled) => {
          setAdded(null)
          turn.mutate({ enabled, ip: d.ip })
        }}
      />
      {d.enabled && (
        <>
          <PrefRow label={t('lan.address')} hint={t('lan.addressHint')}>
            {d.url ? <code className="lan-url">{d.url}</code> : <span className="field-hint">{d.candidates.length === 0 || d.error ? '' : t('lan.starting')}</span>}
          </PrefRow>
          {d.candidates.length === 0 && <p className="field-error">{t('lan.noNetwork')}</p>}
          {d.candidates.length > 0 && d.error && <p className="field-error">{t('lan.bindFailed')}</p>}
          {d.candidates.length > 1 && (
            <PrefRow label={t('lan.network')} htmlFor="lan-ip">
              <span className="select">
                <select id="lan-ip" value={d.ip ?? ''} disabled={turn.isPending} onChange={(e) => turn.mutate({ enabled: true, ip: e.target.value || null })}>
                  <option value="">{t('lan.networkAuto')}</option>
                  {d.candidates.map((c) => (
                    <option key={c.ip} value={c.ip}>
                      {c.name ? `${c.ip} (${c.name})` : c.ip}
                    </option>
                  ))}
                </select>
                <Icon name="down" size={14} />
              </span>
            </PrefRow>
          )}
          <p className="pref-note">
            <Icon name="shield" size={16} />
            {t('lan.firewall')}
          </p>
          {d.pairing ? (
            <PairingBox lan={d} onClose={() => close.mutate()} />
          ) : (
            <div className="pref-actions">
              <button
                type="button"
                className="btn btn-primary btn-s"
                disabled={pairing.isPending || !d.active_ip}
                onClick={() => {
                  setAdded(null)
                  pairing.mutate()
                }}
              >
                <Icon name="plus" size={15} />
                {t('lan.pairStart')}
              </button>
            </div>
          )}
          {added && !d.pairing && (
            <p className="set-saved" role="status">
              <Icon name="check" size={15} stroke={2.2} />
              {t('lan.added', { name: added })}
            </p>
          )}
        </>
      )}
      {(d.enabled || d.devices.length > 0) && (
        <>
          <h3 className="field-label">{t('lan.devices')}</h3>
          {d.devices.length === 0 ? (
            <p className="field-hint">{t('lan.noDevices')}</p>
          ) : (
            <ul className="lan-devices">
              {d.devices.map((x) => (
                <Device key={x.id} d={x} busy={remove.isPending} onRemove={() => remove.mutate(x.id)} />
              ))}
            </ul>
          )}
        </>
      )}
      {failed && (
        <p className="field-error" role="alert">
          {t('pref.failed')}
        </p>
      )}
      {d.enabled && (
        <>
          <p className="field-hint">{t('lan.install')}</p>
          <p className="field-hint">{t('lan.limits')}</p>
        </>
      )}
    </>
  )
}

/** What a paired device sees instead of the PC-only settings. */
export function ThisDevice({ name }: { name: string | null }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const [ask, setAsk] = useState(false)
  const out = useMutation({ mutationFn: sessionApi.logout, onSuccess: () => qc.invalidateQueries() })
  return (
    <>
      <p className="field-hint">{t('lan.thisDeviceText', { name: name ?? '?' })}</p>
      <div className="pref-actions">
        {!ask ? (
          <button type="button" className="btn btn-glass btn-s" onClick={() => setAsk(true)}>
            <Icon name="close" size={15} />
            {t('lan.logout')}
          </button>
        ) : (
          <span className="pref-confirm-row" role="group" aria-label={t('lan.logoutAsk')}>
            <span className="pref-confirm">{t('lan.logoutAsk')}</span>
            <button type="button" className="btn btn-danger btn-s" disabled={out.isPending} autoFocus onClick={() => out.mutate()}>
              {t('lan.logoutYes')}
            </button>
            <button type="button" className="btn btn-glass btn-s" onClick={() => setAsk(false)}>
              {t('common.cancel')}
            </button>
          </span>
        )}
      </div>
    </>
  )
}
