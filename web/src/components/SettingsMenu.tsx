import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { LANGUAGES, useI18n } from '../i18n'
import Icon from './Icon'

/** Settings in the header: a small button with the current language code that opens the panel. */
export default function SettingsMenu() {
  const { lang, t, setLang } = useI18n()
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  const current = LANGUAGES.find((l) => l.id === lang) ?? LANGUAGES[0]

  useEffect(() => {
    if (!open) return
    root.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus()
    const onDown = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      setOpen(false)
      trigger.current?.focus()
    }
    window.addEventListener('pointerdown', onDown)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('pointerdown', onDown)
      window.removeEventListener('keydown', onKey)
    }
  }, [open])

  // Radio group keys: arrows move the choice (and focus), like native radios.
  const onRadioKey = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(e.key)) return
    e.preventDefault()
    const at = LANGUAGES.findIndex((l) => l.id === lang)
    const last = LANGUAGES.length - 1
    const index =
      e.key === 'Home' ? 0 : e.key === 'End' ? last : (at + (e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 1) + LANGUAGES.length) % LANGUAGES.length
    const to = LANGUAGES[index]
    if (!to) return
    setLang(to.id)
    requestAnimationFrame(() => root.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus())
  }

  return (
    <div className="settings" ref={root}>
      <button
        ref={trigger}
        type="button"
        className="settings-btn"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={t('settings.open', { lang: current.name })}
        title={t('settings.title')}
        onClick={() => setOpen((o) => !o)}
      >
        <Icon name="sliders" size={18} />
        <span className="settings-code" aria-hidden="true">
          {current.id.toUpperCase()}
        </span>
      </button>
      {open && (
        <div className="settings-pop" role="dialog" aria-labelledby="settings-title">
          <h2 className="settings-title" id="settings-title">
            {t('settings.title')}
          </h2>
          <p className="field-label" id="settings-lang">
            {t('settings.language')}
          </p>
          <div className="settings-langs" role="radiogroup" aria-labelledby="settings-lang" onKeyDown={onRadioKey}>
            {LANGUAGES.map((l) => (
              <button
                key={l.id}
                type="button"
                role="radio"
                lang={l.locale}
                aria-checked={l.id === lang}
                tabIndex={l.id === lang ? 0 : -1}
                className="settings-lang"
                onClick={() => setLang(l.id)}
              >
                <span className="settings-lang-check">{l.id === lang && <Icon name="check" size={15} stroke={2.2} />}</span>
                {l.name}
              </button>
            ))}
          </div>
          <p className="field-hint">{t('settings.languageHint')}</p>
        </div>
      )}
    </div>
  )
}
