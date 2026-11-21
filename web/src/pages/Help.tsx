import { Link, useNavigate } from 'react-router'
import Icon from '../components/Icon'
import { ProtonSteps } from '../components/SetupParts'
import { startTour } from '../components/Tour'
import { useI18n, type Key } from '../i18n'
import { PLAYER_KEYS } from '../lib/keys'
import { useCoarse, useIsPhone } from '../lib/platform'

/** On a phone the player is used with fingers, so the help shows these instead of the keyboard shortcuts. */
const GESTURES: { id: string; how: Key; what: Key }[] = [
  { id: 'tap', how: 'help.gTap', what: 'help.gTapWhat' },
  { id: 'double', how: 'help.gDouble', what: 'help.gDoubleWhat' },
  { id: 'scrub', how: 'help.gScrub', what: 'help.gScrubWhat' },
  { id: 'subs', how: 'help.gSubs', what: 'help.gSubsWhat' },
]

const FAQ: { q: Key; a: Key }[] = [
  { q: 'help.q1', a: 'help.a1' },
  { q: 'help.q2', a: 'help.a2' },
  { q: 'help.q3', a: 'help.a3' },
  { q: 'help.q4', a: 'help.a4' },
  { q: 'help.q5', a: 'help.a5' },
  { q: 'help.q6', a: 'help.a6' },
]

export default function Help() {
  const { t } = useI18n()
  const navigate = useNavigate()
  const coarse = useCoarse()
  const phone = useIsPhone()
  const touch = coarse || phone
  const keys: { id: string; keys: string; what: string }[] = [
    { id: 'search', keys: '/', what: t('help.keySearch') },
    ...PLAYER_KEYS.map((k) => ({ id: k.id, keys: k.keys.startsWith('vp.') ? t(k.keys as Key) : k.keys, what: t(k.what) })),
  ]
  return (
    <div className="page help">
      <div className="page-head">
        <div>
          <h1 className="page-title">{t('help.title')}</h1>
          <p className="page-sub">{t('help.sub')}</p>
        </div>
        <div className="actions">
          <button
            type="button"
            className="btn btn-glass"
            onClick={() => {
              navigate('/')
              window.setTimeout(startTour, 300)
            }}
          >
            <Icon name="play" size={16} />
            {t('pref.tour')}
          </button>
          <Link to="/einstellungen" className="btn btn-glass">
            <Icon name="sliders" size={16} />
            {t('settings.title')}
          </Link>
        </div>
      </div>
      <div className="help-grid">
        <section className="help-faq" aria-labelledby="help-faq">
          <h2 className="set-title" id="help-faq">
            {t('help.faq')}
          </h2>
          {FAQ.map((f) => (
            <details key={f.q} className="help-item">
              <summary>
                {t(f.q)}
                <Icon name="down" size={16} />
              </summary>
              <p>{t(f.a)}</p>
            </details>
          ))}
          <details className="help-item">
            <summary>
              {t('vpn.howTitle')}
              <Icon name="down" size={16} />
            </summary>
            <ProtonSteps />
          </details>
        </section>
        {touch ? (
          <section className="set-card help-keys" aria-labelledby="help-gestures">
            <h2 className="set-title" id="help-gestures">
              {t('help.gestures')}
            </h2>
            <dl className="vp-help-list help-gestures">
              {GESTURES.map((g) => (
                <div key={g.id}>
                  <dt>{t(g.how)}</dt>
                  <dd>{t(g.what)}</dd>
                </div>
              ))}
            </dl>
          </section>
        ) : (
        <section className="set-card help-keys" aria-labelledby="help-keys">
          <h2 className="set-title" id="help-keys">
            {t('vp.shortcuts')}
          </h2>
          <dl className="vp-help-list">
            {keys.map((k) => (
              <div key={k.id}>
                <dt>
                  {k.keys.split(' / ').map((part, i) => (
                    <span key={part}>
                      {i > 0 && ' '}
                      <kbd>{part}</kbd>
                    </span>
                  ))}
                </dt>
                <dd>{k.what}</dd>
              </div>
            ))}
          </dl>
        </section>
        )}
      </div>
    </div>
  )
}
