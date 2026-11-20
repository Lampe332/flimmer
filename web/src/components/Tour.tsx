import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react'
import { useI18n, type Key } from '../i18n'
import { PLAYER_KEYS } from '../lib/keys'
import { setPrefs } from '../lib/prefs'
import Icon from './Icon'

type Step = { target?: string; title: Key; text: Key; keys?: boolean }

/** Spotlight targets are `data-tour` attributes on real UI (header, hero, nav). Steps without one sit centered. */
const STEPS: Step[] = [
  { target: 'search', title: 'tour.searchTitle', text: 'tour.searchText' },
  { target: 'play', title: 'tour.playTitle', text: 'tour.playText' },
  { title: 'tour.sourcesTitle', text: 'tour.sourcesText' },
  { title: 'tour.keysTitle', text: 'tour.keysText', keys: true },
  { target: 'history', title: 'tour.continueTitle', text: 'tour.continueText' },
  { target: 'downloads', title: 'tour.downloadsTitle', text: 'tour.downloadsText' },
  { target: 'settings', title: 'tour.settingsTitle', text: 'tour.settingsText' },
]
const PAD = 8
const GAP = 14
const CARD_W = 360

const START_EVENT = 'flimmer:tour'

/** Starts the tour from anywhere (settings, help, end of the first-run flow). */
export function startTour(): void {
  window.dispatchEvent(new Event(START_EVENT))
}

/** The first visible element for a target (the header and the phone tab bar both carry nav targets). */
function find(target: string): HTMLElement | null {
  const all = document.querySelectorAll<HTMLElement>(`[data-tour="${target}"]`)
  for (const el of all) {
    const r = el.getBoundingClientRect()
    if (r.width > 0 && r.height > 0) return el
  }
  return null
}

type Box = { top: number; left: number; width: number; height: number }

export default function Tour() {
  const [open, setOpen] = useState(false)
  const [at, setAt] = useState(0)
  const [box, setBox] = useState<Box | null>(null)
  const card = useRef<HTMLDivElement>(null)
  const { t } = useI18n()
  const step: Step = STEPS[at] ?? { title: 'tour.searchTitle', text: 'tour.searchText' }

  useEffect(() => {
    const onStart = () => {
      setAt(0)
      setOpen(true)
    }
    window.addEventListener(START_EVENT, onStart)
    return () => window.removeEventListener(START_EVENT, onStart)
  }, [])

  const close = useCallback(() => {
    setOpen(false)
    setPrefs({ tourDone: true })
  }, [])

  // Measure the target every frame while the step is open: the page may still be laying out (hero loading,
  // header turning solid), and scrolling or resizing moves it too. State only changes when the box does.
  useLayoutEffect(() => {
    if (!open) return
    const first = step.target ? find(step.target) : null
    first?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
    let frame = 0
    let last = ''
    const measure = () => {
      const el = step.target ? find(step.target) : null
      const r = el?.getBoundingClientRect()
      const next = r ? { top: r.top - PAD, left: r.left - PAD, width: r.width + PAD * 2, height: r.height + PAD * 2 } : null
      const key = JSON.stringify(next)
      if (key !== last) {
        last = key
        setBox(next)
      }
      frame = requestAnimationFrame(measure)
    }
    measure()
    return () => cancelAnimationFrame(frame)
  }, [open, step])

  useEffect(() => {
    if (open) card.current?.focus()
  }, [open, at])

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
      else if (e.key === 'ArrowRight') setAt((a) => Math.min(STEPS.length - 1, a + 1))
      else if (e.key === 'ArrowLeft') setAt((a) => Math.max(0, a - 1))
      else return
      e.preventDefault()
      e.stopPropagation()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [open, close])

  if (!open) return null
  const last = at === STEPS.length - 1

  // Card below the spotlight if there is room, else above; centered without a target.
  const vw = window.innerWidth
  const vh = window.innerHeight
  const width = Math.min(CARD_W, vw - 32)
  let cardStyle: CSSProperties = { width }
  if (box) {
    const left = Math.min(Math.max(16, box.left + box.width / 2 - width / 2), vw - width - 16)
    const below = box.top + box.height + GAP
    cardStyle = below + 220 < vh ? { width, left, top: below } : { width, left, bottom: vh - box.top + GAP }
  }

  return (
    <div className="tour" role="presentation">
      {box ? (
        <div className="tour-spot" style={{ top: box.top, left: box.left, width: box.width, height: box.height }} aria-hidden="true" />
      ) : (
        <div className="tour-dim" aria-hidden="true" />
      )}
      <div
        ref={card}
        className={`tour-card${box ? '' : ' is-center'}`}
        style={cardStyle}
        role="dialog"
        aria-modal="true"
        aria-labelledby="tour-title"
        aria-describedby="tour-text"
        tabIndex={-1}
      >
        <p className="tour-count">{t('tour.count', { n: at + 1, total: STEPS.length })}</p>
        <h2 className="tour-title" id="tour-title">
          {t(step.title)}
        </h2>
        <p className="tour-text" id="tour-text">
          {t(step.text)}
        </p>
        {step.keys && (
          <dl className="tour-keys">
            {PLAYER_KEYS.filter((k) => ['play', 'seek', 'full', 'subs', 'next', 'help'].includes(k.id)).map((k) => (
              <div key={k.id}>
                <dt>
                  <kbd>{k.keys.startsWith('vp.') ? t(k.keys as Key) : k.keys}</kbd>
                </dt>
                <dd>{t(k.what)}</dd>
              </div>
            ))}
          </dl>
        )}
        <div className="tour-actions">
          <button type="button" className="link-btn" onClick={close}>
            {t('tour.skip')}
          </button>
          <span className="tour-buttons">
            {at > 0 && (
              <button type="button" className="btn btn-glass btn-s" onClick={() => setAt(at - 1)} aria-label={t('ob.back')}>
                <Icon name="left" size={16} />
              </button>
            )}
            <button type="button" className="btn btn-primary btn-s" onClick={() => (last ? close() : setAt(at + 1))}>
              {last ? t('tour.done') : t('ob.next')}
              {!last && <Icon name="right" size={16} />}
            </button>
          </span>
        </div>
      </div>
    </div>
  )
}
