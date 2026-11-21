import { useState, type ComponentType } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router'
import Icon, { type IconName } from '../components/Icon'
import Sheet, { SheetOption } from '../components/Sheet'
import { LANGUAGES, useI18n, type Key } from '../i18n'
import { useTitle } from '../lib/title'

export type MobileSection = { id: string; label: Key; icon: IconName; summary: Key; Body: ComponentType }

/**
 * Settings on a phone: a grouped list like the system settings. The language is one row that opens a picker sheet;
 * every other group opens as its own page (`?s=player`), so Back returns to the list.
 */
export default function MobileSettings({ sections }: { sections: MobileSection[] }) {
  const { t, lang } = useI18n()
  const [params] = useSearchParams()
  const navigate = useNavigate()
  const open = sections.find((s) => s.id === params.get('s'))
  useTitle(open ? t(open.label) : t('settings.title'))

  if (open) {
    const Body = open.Body
    return (
      <div className="msettings msettings-sub">
        <div className="mpage-bar">
          <button type="button" className="mpage-back" onClick={() => navigate(-1)} aria-label={t('m.back')}>
            <Icon name="arrowLeft" size={22} />
          </button>
          <h1 className="mpage-title">{t(open.label)}</h1>
        </div>
        <Body />
      </div>
    )
  }

  const current = LANGUAGES.find((l) => l.id === lang) ?? LANGUAGES[0]
  return (
    <div className="msettings">
      <h1 className="msettings-title">{t('settings.title')}</h1>
      <p className="msettings-sub-text">{t('pref.sub')}</p>

      <ul className="mlist" aria-label={t('pref.general')}>
        <li>
          <LanguageRow current={current.name} />
        </li>
      </ul>

      <ul className="mlist" aria-label={t('pref.sections')}>
        {sections.map((s) => (
          <li key={s.id}>
            <Link to={`/einstellungen?s=${s.id}`} className="mlist-row">
              <span className="mlist-icon">
                <Icon name={s.icon} size={20} />
              </span>
              <span className="mlist-text">
                <span className="mlist-label">{t(s.label)}</span>
                <span className="mlist-hint">{t(s.summary)}</span>
              </span>
              <Icon name="right" size={18} />
            </Link>
          </li>
        ))}
      </ul>

      <ul className="mlist">
        <li>
          <Link to="/hilfe" className="mlist-row">
            <span className="mlist-icon">
              <Icon name="help" size={20} />
            </span>
            <span className="mlist-text">
              <span className="mlist-label">{t('help.title')}</span>
              <span className="mlist-hint">{t('m.helpSummary')}</span>
            </span>
            <Icon name="right" size={18} />
          </Link>
        </li>
      </ul>
    </div>
  )
}

function LanguageRow({ current }: { current: string }) {
  const { t, lang, setLang } = useI18n()
  const [open, setOpen] = useState(false)
  return (
    <>
      <button type="button" className="mlist-row" onClick={() => setOpen(true)} aria-haspopup="dialog">
        <span className="mlist-icon">
          <Icon name="globe" size={20} />
        </span>
        <span className="mlist-text">
          <span className="mlist-label">{t('settings.language')}</span>
          <span className="mlist-hint">{current}</span>
        </span>
        <Icon name="right" size={18} />
      </button>
      <Sheet open={open} onClose={() => setOpen(false)} title={t('settings.language')}>
        <p className="sheet-hint">{t('settings.languageHint')}</p>
        <div role="menu" aria-label={t('settings.language')}>
          {LANGUAGES.map((l) => (
            <SheetOption
              key={l.id}
              checked={l.id === lang}
              label={l.name}
              lang={l.locale}
              onPick={() => {
                setLang(l.id)
                setOpen(false)
              }}
            />
          ))}
        </div>
      </Sheet>
    </>
  )
}
