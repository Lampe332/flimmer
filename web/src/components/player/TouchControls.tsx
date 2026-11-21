import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import type { T } from '../../i18n'
import { formatClock } from '../../lib/format'
import Icon, { type IconName } from '../Icon'
import Sheet, { SheetOption } from '../Sheet'

/**
 * The player's controls for a finger, after the player of Seanime Mobile:
 * - a tap on the picture shows or hides the bars (never pauses), right away; they hide 5 s after the last touch
 * - a double tap on the left or right half seeks 10 s back or forward, every further quick tap another 10 s
 * - in the middle: 10 s back, play/pause, 10 s forward; top: next episode, subtitles, audio, quality, sound;
 *   bottom: the time (a tap switches to the remaining time), fullscreen and the time bar
 * - the time bar: a tap jumps there, a drag scrubs once the finger moved 10 px sideways, with a big thumb
 * - subtitles, audio and quality open as bottom sheets
 */

export const TOUCH_HIDE_MS = 5000
const DOUBLE_TAP_MS = 280
const SEEK_STEP = 10
const SLOP = 10

export type Choice = { id: number; label: string; hint?: string }
export type MenuId = 'quality' | 'audio' | 'subs'
type Pick = { title: string; icon: IconName; value: number; options: Choice[]; onPick: (id: number) => void }

type Props = {
  t: T
  title: string
  subtitle?: string
  playing: boolean
  /** Loading or not started yet: the spinner takes the middle. */
  busy: boolean
  time: number
  duration: number
  buffered: [number, number][]
  ahead: { from: number; seconds: number } | null
  status?: string
  /** The bars are visible (the player's own idle logic). */
  shown: boolean
  show: () => void
  hide: () => void
  toggle: () => void
  seekTo: (seconds: number) => void
  nowAt: () => number
  scrub: number | null
  setScrub: (s: number | null) => void
  fullscreen: boolean
  toggleFullscreen: () => void
  next?: { label: string; go: () => void }
  muted: boolean
  toggleMute: () => void
  menu: MenuId | null
  setMenu: (m: MenuId | null) => void
  quality: Pick | null
  audio: Pick | null
  subs: Pick | null
}

type Side = 'left' | 'right'

export default function TouchControls(p: Props) {
  const { t } = p
  const box = useRef<HTMLDivElement>(null)
  const tap = useRef<{ x: number; y: number; at: number; id: number } | null>(null)
  const lastTap = useRef<{ side: Side; at: number; wasShown: boolean } | null>(null)
  const [ripple, setRipple] = useState<{ side: Side; total: number; key: number } | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const [remaining, setRemaining] = useState(false)

  useEffect(() => {
    if (!ripple) return
    const timer = window.setTimeout(() => setRipple(null), 700)
    return () => window.clearTimeout(timer)
  }, [ripple])
  useEffect(() => {
    if (!message) return
    const timer = window.setTimeout(() => setMessage(null), 1800)
    return () => window.clearTimeout(timer)
  }, [message])

  // ---- taps on the picture ----
  const onDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (!e.isPrimary) return
    tap.current = { x: e.clientX, y: e.clientY, at: e.timeStamp, id: e.pointerId }
  }
  const onUp = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = tap.current
    tap.current = null
    if (!d || d.id !== e.pointerId) return
    if (Math.abs(e.clientX - d.x) > SLOP || Math.abs(e.clientY - d.y) > SLOP || e.timeStamp - d.at > 500) return
    const r = e.currentTarget.getBoundingClientRect()
    const side: Side = e.clientX - r.left < r.width / 2 ? 'left' : 'right'
    const last = lastTap.current
    const now = e.timeStamp
    if (last && last.side === side && now - last.at < DOUBLE_TAP_MS) {
      // double tap (and every further quick tap): seek, the bars stay as they were before the first tap
      const step = side === 'left' ? -SEEK_STEP : SEEK_STEP
      p.seekTo(p.nowAt() + step)
      setRipple((r0) => ({ side, total: r0 && r0.side === side ? r0.total + SEEK_STEP : SEEK_STEP, key: now }))
      if (last.wasShown) p.show()
      else p.hide()
      lastTap.current = { side, at: now, wasShown: last.wasShown }
      return
    }
    lastTap.current = { side, at: now, wasShown: p.shown }
    if (p.shown) p.hide()
    else p.show()
  }

  // ---- time bar ----
  const bar = useRef<HTMLDivElement>(null)
  const drag = useRef<{ x: number; y: number; scrubbing: boolean } | null>(null)
  const ratio = (clientX: number) => {
    const r = bar.current?.getBoundingClientRect()
    return r && r.width > 0 ? Math.min(1, Math.max(0, (clientX - r.left) / r.width)) : 0
  }
  const barDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (p.duration <= 0) return
    e.currentTarget.setPointerCapture(e.pointerId)
    drag.current = { x: e.clientX, y: e.clientY, scrubbing: false }
    p.show()
  }
  const barMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current
    if (!d) return
    if (!d.scrubbing) {
      if (Math.abs(e.clientY - d.y) > SLOP && Math.abs(e.clientY - d.y) > Math.abs(e.clientX - d.x)) {
        drag.current = null
        return
      }
      if (Math.abs(e.clientX - d.x) < SLOP) return
      d.scrubbing = true
    }
    p.setScrub(ratio(e.clientX) * p.duration)
  }
  const barUp = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current
    drag.current = null
    if (!d) return
    p.seekTo(ratio(e.clientX) * p.duration)
    p.setScrub(null)
    p.show()
  }
  const barCancel = () => {
    drag.current = null
    p.setScrub(null)
  }

  const shownTime = p.scrub ?? p.time
  const pct = (s: number) => (p.duration > 0 ? Math.min(100, Math.max(0, (s / p.duration) * 100)) : 0)
  const keep = () => p.show()
  const open = p.menu ? p[p.menu] : null
  // the sheet keeps its last content while it slides out
  const lastPick = useRef(open)
  if (open) lastPick.current = open
  const sheetPick = open ?? lastPick.current

  return (
    <div ref={box} className={`vt${p.shown ? ' is-shown' : ''}`}>
      <div
        className="vt-surface"
        onPointerDown={onDown}
        onPointerUp={onUp}
        onPointerCancel={() => (tap.current = null)}
        aria-hidden="true"
      />

      {ripple && (
        <div className={`vt-ripple is-${ripple.side}`} key={ripple.key} aria-live="polite">
          <Icon name={ripple.side === 'left' ? 'back10' : 'fwd10'} size={20} />
          {t(ripple.side === 'left' ? 'm.seekBack' : 'm.seekFwd', { n: ripple.total })}
        </div>
      )}
      {message && (
        <p className="vt-message" role="status">
          {message}
        </p>
      )}

      <div className="vt-top" onPointerDownCapture={keep}>
        {p.next && (
          <button type="button" className="vt-btn" onClick={p.next.go} aria-label={t('vp.nextLabel', { label: p.next.label })}>
            <Icon name="next" size={22} />
          </button>
        )}
        <div className="vt-title">
          <p className="vt-title-main">{p.title}</p>
          {p.subtitle && <p className="vt-title-sub">{p.subtitle}</p>}
        </div>
        {p.subs && (
          <button type="button" className="vt-btn" onClick={() => p.setMenu('subs')} aria-label={p.subs.title} aria-haspopup="dialog">
            <Icon name="subtitles" size={22} />
          </button>
        )}
        {p.audio && (
          <button type="button" className="vt-btn" onClick={() => p.setMenu('audio')} aria-label={p.audio.title} aria-haspopup="dialog">
            <Icon name="audio" size={22} />
          </button>
        )}
        {p.quality && (
          <button type="button" className="vt-btn" onClick={() => p.setMenu('quality')} aria-label={p.quality.title} aria-haspopup="dialog">
            <Icon name="layers" size={22} />
          </button>
        )}
        <button
          type="button"
          className="vt-btn"
          onClick={() => {
            setMessage(p.muted ? t('m.soundOn') : t('m.soundOff'))
            p.toggleMute()
          }}
          aria-label={p.muted ? t('vp.unmute') : t('vp.mute')}
          aria-pressed={p.muted}
        >
          <Icon name={p.muted ? 'mute' : 'volume'} size={22} />
        </button>
      </div>

      <div className="vt-mid" onPointerDownCapture={keep}>
        <button type="button" className="vt-seek" onClick={() => p.seekTo(p.nowAt() - SEEK_STEP)} aria-label={t('m.seekBackLabel')}>
          <Icon name="back10" size={26} />
          <span aria-hidden="true">{SEEK_STEP}</span>
        </button>
        {p.busy ? (
          <span className="vt-play is-spacer" aria-hidden="true" />
        ) : (
          <button type="button" className="vt-play" onClick={p.toggle} aria-label={p.playing ? t('vp.pause') : t('common.play')}>
            <Icon name={p.playing ? 'pause' : 'play'} size={30} />
          </button>
        )}
        <button type="button" className="vt-seek" onClick={() => p.seekTo(p.nowAt() + SEEK_STEP)} aria-label={t('m.seekFwdLabel')}>
          <Icon name="fwd10" size={26} />
          <span aria-hidden="true">{SEEK_STEP}</span>
        </button>
      </div>

      <div className="vt-bottom" onPointerDownCapture={keep}>
        <div className="vt-row">
          <button type="button" className="vt-time" onClick={() => setRemaining((r) => !r)} aria-label={t('m.timeToggle')}>
            {remaining ? `-${formatClock(p.duration - shownTime)}` : formatClock(shownTime)}
            <span className="vt-time-total"> / {formatClock(p.duration)}</span>
          </button>
          {p.status && <span className="vt-status">{p.status}</span>}
          <span className="vt-spacer" />
          <button
            type="button"
            className="vt-btn"
            onClick={p.toggleFullscreen}
            aria-label={p.fullscreen ? t('vp.exitFullscreen') : t('vp.fullscreen')}
          >
            <Icon name={p.fullscreen ? 'shrink' : 'expand'} size={22} />
          </button>
        </div>
        <div
          ref={bar}
          className={`vt-bar${p.scrub !== null ? ' is-scrubbing' : ''}`}
          role="slider"
          tabIndex={0}
          aria-label={t('vp.position')}
          aria-valuemin={0}
          aria-valuemax={Math.round(p.duration)}
          aria-valuenow={Math.round(shownTime)}
          aria-valuetext={t('vp.positionText', { a: formatClock(shownTime), b: formatClock(p.duration) })}
          onPointerDown={barDown}
          onPointerMove={barMove}
          onPointerUp={barUp}
          onPointerCancel={barCancel}
        >
          <div className="vt-track">
            {p.ahead && <span className="vt-ahead" style={{ left: `${pct(p.ahead.from)}%`, width: `${pct(p.ahead.seconds)}%` }} />}
            {p.buffered.map(([a, z]) => (
              <span key={a} className="vt-buffered" style={{ left: `${pct(a)}%`, width: `${pct(z - a)}%` }} />
            ))}
            <span className="vt-played" style={{ transform: `scaleX(${pct(shownTime) / 100})` }} />
          </div>
          <span className="vt-thumb" style={{ left: `${pct(shownTime)}%` }} />
          {p.scrub !== null && (
            <span className="vt-tip" style={{ left: `${Math.min(90, Math.max(10, pct(p.scrub)))}%` }}>
              {formatClock(p.scrub)}
            </span>
          )}
        </div>
      </div>

      <Sheet
        open={!!open}
        onClose={() => p.setMenu(null)}
        title={sheetPick?.title ?? ''}
        inline={p.fullscreen}
        className="vt-sheet"
      >
        <div role="menu" aria-label={sheetPick?.title}>
          {sheetPick?.options.map((o) => (
            <SheetOption
              key={o.id}
              checked={o.id === sheetPick?.value}
              label={o.label}
              hint={o.hint}
              onPick={() => {
                sheetPick?.onPick(o.id)
                p.setMenu(null)
              }}
            />
          ))}
        </div>
      </Sheet>
    </div>
  )
}
