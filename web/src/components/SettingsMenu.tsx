import { NavLink } from 'react-router'
import { LANGUAGES, useI18n } from '../i18n'
import Icon from './Icon'

/** Settings in the header: a small button with the current language code that opens the settings page. */
export default function SettingsMenu() {
  const { lang, t } = useI18n()
  const current = LANGUAGES.find((l) => l.id === lang) ?? LANGUAGES[0]
  return (
    <div className="settings">
      <NavLink
        to="/einstellungen"
        className="settings-btn"
        data-tour="settings"
        aria-label={t('settings.open', { lang: current.name })}
        title={t('settings.title')}
      >
        <Icon name="sliders" size={18} />
        <span className="settings-code" aria-hidden="true">
          {current.id.toUpperCase()}
        </span>
      </NavLink>
    </div>
  )
}
