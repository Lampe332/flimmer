import { useEffect, useId, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { useI18n, type Key } from '../i18n'
import { SetupError, readTextFile, setupApi, type Page, type TokenError } from '../lib/setup'
import Icon from './Icon'

/** Opens one of the fixed help pages in the system browser. */
export function OpenPage({ page, label, quiet }: { page: Page; label: string; quiet?: boolean }) {
  const [failed, setFailed] = useState(false)
  const { t } = useI18n()
  return (
    <span className="open-page">
      <button
        type="button"
        className={quiet ? 'link-btn' : 'btn btn-glass btn-s'}
        onClick={() => {
          setFailed(false)
          setupApi.open(page).catch(() => setFailed(true))
        }}
      >
        {label}
        <Icon name="right" size={14} />
      </button>
      {failed && <span className="field-error">{t('ob.openFailed')}</span>}
    </span>
  )
}

const TOKEN_ERRORS: Record<TokenError, Key> = {
  format: 'ob.tokenFormat',
  api_key: 'ob.tokenApiKey',
  rejected: 'ob.tokenRejected',
  unreachable: 'ob.tokenUnreachable',
  other: 'ob.tokenOther',
}

type Check = { state: 'idle' } | { state: 'checking' } | { state: 'ok' } | { state: 'error'; code: TokenError }

/**
 * Paste field for the TMDB token. Checks it with TMDB as soon as it looks complete (and on the button);
 * the server only stores it when TMDB accepts it. The field never shows a stored token.
 */
export function TokenField({ onSaved, autoFocus }: { onSaved?: () => void; autoFocus?: boolean }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const id = useId()
  const [value, setValue] = useState('')
  const [check, setCheck] = useState<Check>({ state: 'idle' })
  const [shown, setShown] = useState(false)
  const run = useRef(0)
  const saved = useRef(onSaved)
  saved.current = onSaved

  const submit = async (raw: string) => {
    const token = raw.replace(/\s+/g, '')
    if (!token) return
    const mine = ++run.current
    setCheck({ state: 'checking' })
    try {
      await setupApi.token(token)
      if (mine !== run.current) return
      setCheck({ state: 'ok' })
      setValue('')
      await qc.invalidateQueries()
      saved.current?.()
    } catch (e) {
      if (mine !== run.current) return
      setCheck({ state: 'error', code: e instanceof SetupError ? e.code : 'other' })
    }
  }

  // A pasted token is checked right away; typing waits for a pause.
  useEffect(() => {
    const token = value.replace(/\s+/g, '')
    if (token.length < 100) return
    const timer = window.setTimeout(() => void submit(token), 500)
    return () => window.clearTimeout(timer)
  }, [value]) // submit only reads refs and setters

  return (
    <form
      className="token"
      onSubmit={(e) => {
        e.preventDefault()
        void submit(value)
      }}
    >
      <label className="field-label" htmlFor={id}>
        {t('ob.tokenLabel')}
      </label>
      <div className="token-row">
        <input
          id={id}
          className="input token-input"
          type={shown ? 'text' : 'password'}
          value={value}
          autoFocus={autoFocus}
          autoComplete="off"
          spellCheck={false}
          placeholder="eyJ…"
          aria-describedby={`${id}-status`}
          aria-invalid={check.state === 'error'}
          onChange={(e) => {
            setValue(e.target.value)
            if (check.state !== 'checking') setCheck({ state: 'idle' })
          }}
        />
        <button type="button" className="btn btn-glass btn-s" onClick={() => setShown((s) => !s)} aria-pressed={shown}>
          {shown ? t('ob.hide') : t('ob.show')}
        </button>
        <button type="submit" className="btn btn-primary btn-s" disabled={!value.trim() || check.state === 'checking'}>
          {t('ob.check')}
        </button>
      </div>
      <p id={`${id}-status`} className={`token-status is-${check.state}`} role="status" aria-live="polite">
        {check.state === 'checking' && (
          <>
            <span className="spinner" aria-hidden="true" />
            {t('ob.checking')}
          </>
        )}
        {check.state === 'ok' && (
          <>
            <Icon name="check" size={16} stroke={2.4} />
            {t('ob.tokenOk')}
          </>
        )}
        {check.state === 'error' && (
          <>
            <Icon name="warn" size={16} />
            {t(TOKEN_ERRORS[check.code])}
          </>
        )}
        {check.state === 'idle' && t('ob.tokenPrivacy')}
      </p>
    </form>
  )
}

type VpnState = { state: 'idle' } | { state: 'busy' } | { state: 'ok' } | { state: 'error'; text: string }

/** Picks a WireGuard .conf and hands it to the server, which checks and keeps it. The content is never shown. */
export function VpnImport({ has, onChange }: { has: boolean; onChange?: () => void }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const input = useRef<HTMLInputElement>(null)
  const [st, setSt] = useState<VpnState>({ state: 'idle' })

  const pick = async (file: File | undefined) => {
    if (!file) return
    setSt({ state: 'busy' })
    try {
      const text = await readTextFile(file, 8 * 1024)
      await setupApi.vpnImport(text)
      setSt({ state: 'ok' })
      await qc.invalidateQueries({ queryKey: ['setup'] })
      await qc.invalidateQueries({ queryKey: ['torrents'] })
      onChange?.()
    } catch {
      setSt({ state: 'error', text: t('ob.vpnBad') })
    } finally {
      if (input.current) input.current.value = ''
    }
  }

  return (
    <div className="vpn-import">
      <input
        ref={input}
        type="file"
        accept=".conf,text/plain"
        className="sr-only"
        tabIndex={-1}
        aria-hidden="true"
        onChange={(e) => void pick(e.target.files?.[0])}
      />
      <div className="vpn-import-row">
        <span className={`set-found${has ? ' is-ok' : ' is-bad'}`}>
          <Icon name={has ? 'check' : 'warn'} size={14} stroke={2.2} />
          {has ? t('set.fileOk') : t('set.fileMissing')}
        </span>
        <button type="button" className="btn btn-glass btn-s" disabled={st.state === 'busy'} onClick={() => input.current?.click()}>
          <Icon name="folder" size={15} />
          {has ? t('ob.vpnReplace') : t('ob.vpnPick')}
        </button>
      </div>
      <p className={`token-status is-${st.state === 'busy' ? 'checking' : st.state}`} role="status" aria-live="polite">
        {st.state === 'ok' && (
          <>
            <Icon name="check" size={16} stroke={2.4} />
            {t('ob.vpnOk')}
          </>
        )}
        {st.state === 'error' && (
          <>
            <Icon name="warn" size={16} />
            {st.text}
          </>
        )}
      </p>
    </div>
  )
}
