import { useEffect, useRef, useState, type ReactNode, type RefObject } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import Icon from '../components/Icon'
import { Mark } from '../components/Logo'
import { OpenPage, TokenField, VpnImport } from '../components/SetupParts'
import { LANGUAGES, useI18n, type Key } from '../i18n'
import { ApiError } from '../lib/api'
import { setupApi, type SetupState } from '../lib/setup'
import { torrentApi } from '../lib/torrents'

type Step = 'welcome' | 'tmdb' | 'vpn' | 'folders' | 'done'
const STEPS: { id: Step; label: Key }[] = [
  { id: 'welcome', label: 'ob.navWelcome' },
  { id: 'tmdb', label: 'ob.navTmdb' },
  { id: 'vpn', label: 'ob.navVpn' },
  { id: 'folders', label: 'ob.navFolders' },
  { id: 'done', label: 'ob.navDone' },
]

/**
 * First-run setup, full screen: language, TMDB token (required), Proton WireGuard file (optional),
 * download folders. Shown until finished; settings can open it again (`reopened`).
 */
export default function Onboarding({ state, onFinish }: { state: SetupState; onFinish: () => void }) {
  const { t } = useI18n()
  const [step, setStep] = useState<Step>('welcome')
  const at = STEPS.findIndex((s) => s.id === step)
  const heading = useRef<HTMLHeadingElement>(null)
  const go = (to: Step) => setStep(to)

  // Each step starts with its heading focused, so screen readers and keyboard users land at the top.
  useEffect(() => {
    heading.current?.focus()
  }, [step])

  return (
    <div className="ob">
      <div className="ob-glow" aria-hidden="true" />
      <header className="ob-top">
        <span className="ob-brand">
          <Mark size={26} />
          Flimmer
        </span>
        <ol className="ob-steps" aria-label={t('ob.progress', { n: at + 1, total: STEPS.length })}>
          {STEPS.map((s, i) => (
            <li key={s.id} className={i < at ? 'is-done' : i === at ? 'is-now' : ''} aria-current={i === at ? 'step' : undefined}>
              <span className="ob-step-dot" aria-hidden="true">
                {i < at ? <Icon name="check" size={12} stroke={2.6} /> : i + 1}
              </span>
              <span className="ob-step-label">{t(s.label)}</span>
            </li>
          ))}
        </ol>
      </header>

      <main className="ob-card" key={step}>
        {step === 'welcome' && <Welcome heading={heading} onNext={() => go('tmdb')} />}
        {step === 'tmdb' && <Tmdb heading={heading} has={state.tmdb === 'set'} onBack={() => go('welcome')} onNext={() => go('vpn')} />}
        {step === 'vpn' && <Vpn heading={heading} has={state.vpn_file} onBack={() => go('tmdb')} onNext={() => go('folders')} />}
        {step === 'folders' && <Folders heading={heading} state={state} onBack={() => go('vpn')} onNext={() => go('done')} />}
        {step === 'done' && <Done heading={heading} onBack={() => go('folders')} onFinish={onFinish} />}
      </main>
    </div>
  )
}

type HeadingRef = { heading: RefObject<HTMLHeadingElement | null> }

function Nav({ onBack, next }: { onBack?: () => void; next: ReactNode }) {
  const { t } = useI18n()
  return (
    <div className="ob-nav">
      {onBack ? (
        <button type="button" className="btn btn-glass" onClick={onBack}>
          <Icon name="left" size={16} />
          {t('ob.back')}
        </button>
      ) : (
        <span />
      )}
      {next}
    </div>
  )
}

function Welcome({ heading, onNext }: HeadingRef & { onNext: () => void }) {
  const { t, lang, setLang } = useI18n()
  return (
    <>
      <p className="ob-kicker">{t('ob.welcomeKicker')}</p>
      <h1 className="ob-title" ref={heading} tabIndex={-1}>
        {t('ob.welcomeTitle')}
      </h1>
      <p className="ob-lead">{t('ob.welcomeText')}</p>
      <p className="field-label" id="ob-lang">
        {t('settings.language')}
      </p>
      <div className="ob-langs" role="radiogroup" aria-labelledby="ob-lang">
        {LANGUAGES.map((l) => (
          <button
            key={l.id}
            type="button"
            role="radio"
            lang={l.locale}
            aria-checked={l.id === lang}
            className="ob-lang"
            onClick={() => setLang(l.id)}
          >
            {l.name}
            {l.id === lang && <Icon name="check" size={15} stroke={2.4} />}
          </button>
        ))}
      </div>
      <Nav
        next={
          <button type="button" className="btn btn-primary" onClick={onNext}>
            {t('ob.start')}
            <Icon name="right" size={16} />
          </button>
        }
      />
    </>
  )
}

function Tmdb({ heading, has, onBack, onNext }: HeadingRef & { has: boolean; onBack: () => void; onNext: () => void }) {
  const { t } = useI18n()
  const [ok, setOk] = useState(has)
  const [replace, setReplace] = useState(false)
  return (
    <>
      <p className="ob-kicker">{t('ob.tmdbKicker')}</p>
      <h1 className="ob-title" ref={heading} tabIndex={-1}>
        {t('ob.tmdbTitle')}
      </h1>
      <p className="ob-lead">{t('ob.tmdbText')}</p>
      <ol className="ob-howto">
        <li>
          <span>{t('ob.tmdbStep1')}</span>
          <OpenPage page="tmdb_signup" label={t('ob.tmdbSignup')} />
        </li>
        <li>
          <span>{t('ob.tmdbStep2')}</span>
          <OpenPage page="tmdb_api" label={t('ob.tmdbApiPage')} />
        </li>
        <li>
          <span>{t('ob.tmdbStep3')}</span>
        </li>
        <li>
          <span>{t('ob.tmdbStep4')}</span>
        </li>
      </ol>
      {has && !replace ? (
        <div className="ob-saved">
          <span className="token-status is-ok">
            <Icon name="check" size={16} stroke={2.4} />
            {t('ob.tokenStored')}
          </span>
          <button type="button" className="link-btn" onClick={() => setReplace(true)}>
            {t('ob.tokenReplace')}
          </button>
        </div>
      ) : (
        <TokenField autoFocus={!has} onSaved={() => setOk(true)} />
      )}
      <Nav
        onBack={onBack}
        next={
          <button type="button" className="btn btn-primary" disabled={!ok} onClick={onNext}>
            {t('ob.next')}
            <Icon name="right" size={16} />
          </button>
        }
      />
    </>
  )
}

function Vpn({ heading, has, onBack, onNext }: HeadingRef & { has: boolean; onBack: () => void; onNext: () => void }) {
  const { t } = useI18n()
  const [imported, setImported] = useState(has)
  return (
    <>
      <p className="ob-kicker">{t('ob.vpnKicker')}</p>
      <h1 className="ob-title" ref={heading} tabIndex={-1}>
        {t('ob.vpnTitle')}
      </h1>
      <p className="ob-lead">{t('ob.vpnText')}</p>
      <ol className="ob-howto">
        <li>
          <span>{t('ob.vpnStep1')}</span>
        </li>
        <li>
          <span>{t('ob.vpnStep2')}</span>
          <OpenPage page="proton_wireguard" label={t('ob.vpnProton')} />
        </li>
        <li>
          <span>{t('ob.vpnStep3')}</span>
        </li>
        <li>
          <span>{t('ob.vpnStep4')}</span>
        </li>
      </ol>
      <VpnImport has={imported} onChange={() => setImported(true)} />
      <Nav
        onBack={onBack}
        next={
          imported ? (
            <button type="button" className="btn btn-primary" onClick={onNext}>
              {t('ob.next')}
              <Icon name="right" size={16} />
            </button>
          ) : (
            <button type="button" className="btn btn-glass" onClick={onNext}>
              {t('ob.skip')}
              <Icon name="right" size={16} />
            </button>
          )
        }
      />
    </>
  )
}

function Folders({ heading, state, onBack, onNext }: HeadingRef & { state: SetupState; onBack: () => void; onNext: () => void }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const saved = useQuery({ queryKey: ['torrent-settings'], queryFn: torrentApi.settings, retry: false })
  // A fresh install still has the developer default (another drive): suggest the user's Videos folder instead.
  const fresh = !state.onboarded
  const [movies, setMovies] = useState<string | null>(null)
  const [series, setSeries] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const m = movies ?? (fresh ? state.defaults.movies_dir : saved.data?.movies_dir) ?? ''
  const s = series ?? (fresh ? state.defaults.series_dir : saved.data?.series_dir) ?? ''

  const save = async () => {
    if (!saved.data) return
    setBusy(true)
    setError(null)
    try {
      const next = { ...saved.data, movies_dir: m.trim(), series_dir: s.trim() }
      await torrentApi.settingsPut(next)
      qc.setQueryData(['torrent-settings'], next)
      onNext()
    } catch (e) {
      setError(e instanceof ApiError && /absolute/.test(e.message) ? t('ob.folderAbsolute') : t('set.saveFailed'))
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <p className="ob-kicker">{t('ob.foldersKicker')}</p>
      <h1 className="ob-title" ref={heading} tabIndex={-1}>
        {t('ob.foldersTitle')}
      </h1>
      <p className="ob-lead">{t('ob.foldersText')}</p>
      <form
        className="ob-folders"
        onSubmit={(e) => {
          e.preventDefault()
          void save()
        }}
      >
        <label className="field">
          <span className="field-label">{t('set.moviesDir')}</span>
          <input className="input" value={m} onChange={(e) => setMovies(e.target.value)} spellCheck={false} />
        </label>
        <label className="field">
          <span className="field-label">{t('set.seriesDir')}</span>
          <input className="input" value={s} onChange={(e) => setSeries(e.target.value)} spellCheck={false} />
        </label>
        <p className="field-hint">{t('ob.foldersHint')}</p>
        {error && (
          <p className="field-error" role="alert">
            {error}
          </p>
        )}
        <Nav
          onBack={onBack}
          next={
            <button type="submit" className="btn btn-primary" disabled={busy || !saved.data || !m.trim() || !s.trim()}>
              {t('ob.next')}
              <Icon name="right" size={16} />
            </button>
          }
        />
      </form>
    </>
  )
}

function Done({ heading, onBack, onFinish }: HeadingRef & { onBack: () => void; onFinish: () => void }) {
  const { t } = useI18n()
  const [busy, setBusy] = useState(false)
  return (
    <>
      <span className="ob-done-mark" aria-hidden="true">
        <Icon name="check" size={30} stroke={2.4} />
      </span>
      <h1 className="ob-title" ref={heading} tabIndex={-1}>
        {t('ob.doneTitle')}
      </h1>
      <p className="ob-lead">{t('ob.doneText')}</p>
      <Nav
        onBack={onBack}
        next={
          <button
            type="button"
            className="btn btn-primary btn-lg"
            disabled={busy}
            onClick={() => {
              setBusy(true)
              setupApi
                .done()
                .then(onFinish)
                .catch(() => setBusy(false))
            }}
          >
            {t('ob.finish')}
            <Icon name="right" size={16} />
          </button>
        }
      />
    </>
  )
}
