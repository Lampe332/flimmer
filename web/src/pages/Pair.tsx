import { useEffect, useRef, useState, type FormEvent } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import Icon from '../components/Icon'
import Logo from '../components/Logo'
import { LANGUAGES, useI18n, type Key } from '../i18n'
import { PairFailed, sessionApi, type PairError, type Session } from '../lib/session'

const ERRORS: Record<PairError, Key> = {
  wrong_pin: 'pair.wrong',
  locked: 'pair.locked',
  closed: 'pair.closed',
  full: 'pair.full',
  offline: 'pair.offline',
  other: 'pair.other',
}

/** The one-time token of a scanned QR code (`/pair#t=…`). Taken out of the address at once. */
function tokenFromAddress(): string | null {
  const m = /^#t=([0-9a-f]{16,64})$/.exec(window.location.hash)
  if (!m) return null
  window.history.replaceState(null, '', '/')
  return m[1] ?? null
}

/**
 * Shown to a browser the PC does not know yet: a phone or tablet in the home network (or a browser on the PC).
 * It trades the PIN from the PC's settings, or the QR code's token, for a device cookie.
 */
export default function Pair({ session }: { session: Session }) {
  const { t, lang, setLang } = useI18n()
  const qc = useQueryClient()
  const [pin, setPin] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<PairFailed | null>(null)
  const scanned = useRef(false)
  const input = useRef<HTMLInputElement>(null)

  const run = async (go: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    try {
      await go()
      window.history.replaceState(null, '', '/')
      await qc.invalidateQueries()
    } catch (e) {
      setError(e instanceof PairFailed ? e : new PairFailed('other'))
      setPin('')
      input.current?.focus()
    } finally {
      setBusy(false)
    }
  }

  useEffect(() => {
    if (scanned.current) return
    scanned.current = true
    const token = tokenFromAddress()
    if (token) void run(() => sessionApi.pairToken(token))
    // Runs once: the token in the address works a single time.
  }, [])

  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (pin.length === 6 && !busy) void run(() => sessionApi.pairPin(pin))
  }

  return (
    <div className="ob pair">
      <div className="ob-glow" aria-hidden="true" />
      <header className="ob-top">
        <span className="ob-brand">
          <Logo />
        </span>
        <span className="select pair-lang">
          <select aria-label={t('settings.language')} value={lang} onChange={(e) => setLang(e.target.value as typeof lang)}>
            {LANGUAGES.map((l) => (
              <option key={l.id} value={l.id} lang={l.locale}>
                {l.name}
              </option>
            ))}
          </select>
          <Icon name="down" size={14} />
        </span>
      </header>
      <main className="ob-card">
        <p className="ob-kicker">{t('lan.title')}</p>
        <h1 className="ob-title">{t('pair.title')}</h1>
        <p className="ob-lead">{t('pair.lead')}</p>
        <ol className="ob-howto">
          <li>
            <span>{t('pair.step1')}</span>
          </li>
          <li>
            <span>{t('pair.step2')}</span>
          </li>
          <li>
            <span>{t('pair.step3')}</span>
          </li>
        </ol>
        <form className="pair-form" onSubmit={submit}>
          <label className="field">
            <span className="field-label">{t('pair.pin')}</span>
            <input
              ref={input}
              className="input pair-pin"
              inputMode="numeric"
              autoComplete="one-time-code"
              pattern="[0-9]{6}"
              maxLength={6}
              placeholder="000000"
              value={pin}
              onChange={(e) => setPin(e.target.value.replace(/\D/g, '').slice(0, 6))}
              aria-invalid={error ? true : undefined}
              aria-describedby={error ? 'pair-error' : undefined}
              autoFocus
            />
          </label>
          <button type="submit" className="btn btn-primary" disabled={pin.length !== 6 || busy}>
            {busy ? t('pair.connecting') : t('pair.connect')}
          </button>
        </form>
        {error && (
          <p className="field-error" role="alert" id="pair-error">
            {t(ERRORS[error.code], { m: Math.max(1, Math.ceil(error.wait / 60)) })}
          </p>
        )}
        {session.via === 'local' && <p className="field-hint">{t('pair.localHint')}</p>}
      </main>
    </div>
  )
}

/** A paired device opened Flimmer before the PC finished its first-run setup. */
export function SetupOnPc() {
  const { t } = useI18n()
  return (
    <div className="state">
      <h1>{t('pair.setupFirst')}</h1>
      <p>{t('pair.setupFirstText')}</p>
      <div className="state-actions">
        <button type="button" className="btn btn-primary" onClick={() => window.location.reload()}>
          <Icon name="refresh" size={16} />
          {t('err.reload')}
        </button>
      </div>
    </div>
  )
}
