import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from 'react'
import type { T } from '../../i18n'
import { formatClock } from '../../lib/format'
import Icon, { type IconName } from '../Icon'
import Sheet, { SheetOption } from '../Sheet'
import {
  HIDE_MS,
  HUD_MS,
  LONG_PRESS_MS,
  LONG_PRESS_RATE,
  RIPPLE_MS,
  SEEK_BACK,
  SEEK_FWD,
  SLOP,
  SWIPE_MIN,
  SWIPE_RANGE,
  SWIPE_RATIO,
  TAP_MS,
  WIDE_MIN_WIDTH,
  clamp01,
  controlSizes,
  fitBarButton,
  hasScreenBrightness,
  readScreenBrightness,
  writeScreenBrightness,
} from './phone/controls'
import SettingsMenu from './phone/SettingsMenu'

/**
 * The player's controls for a finger, after the phone player of Seanime Mobile (sizes and rules: ./phone/controls.ts):
 * - a tap on the picture shows or hides the bars after TAP_MS (so a double tap does not also toggle them);
 *   they hide 5 s after the last touch while playing, never while paused or with a sheet open
 * - a double tap on the left half goes 5 s back, on the right 10 s forward; every further quick tap adds up
 * - holding the picture plays at 2x until the finger lifts; swiping up or down changes the brightness (left)
 *   or the volume (right)
 * - top: back, title (from 560 px), settings, quality, subtitles, audio, sound; middle: 5 s back, play/pause,
 *   10 s forward; bottom: the time, next episode, fullscreen and the time bar
 */

export const TOUCH_HIDE_MS = HIDE_MS

export type Choice = { id: number; label: string; hint?: string }
export type MenuId = 'quality' | 'audio' | 'subs' | 'settings'
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
  /** The current volume (0 when muted) and a way to set it. */
  volume: number
  setVolume: (v: number) => void
  media: () => HTMLVideoElement | null
  menu: MenuId | null
  setMenu: (m: MenuId | null) => void
  quality: Pick | null
  audio: Pick | null
  subs: Pick | null
  /** Text subtitles exist / one is on (the delay and style entries). */
  hasSubs: boolean
  subOn: boolean
  cancelNext?: () => void
  /** Without app history the back button goes here (the title page), not home. */
  backTo?: string
}

type Side = 'left' | 'right'
type Gesture = {
  id: number
  x: number
  y: number
  side: Side
  /** moved past SLOP: no tap, no long press any more */
  moved: boolean
  mode: 'pending' | 'swipe' | 'long'
  kind?: 'volume' | 'brightness'
  from: number
  height: number
  rate: number
}

export default function TouchControls(p: Props) {
  const { t } = p
  const box = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ w: 412, h: 915 })
  const [ripple, setRipple] = useState<{ side: Side; total: number; key: number } | null>(null)
  const [hud, setHud] = useState<{ text: string; label?: string; level?: number } | null>(null)
  const [remaining, setRemaining] = useState(false)
  /** Browser only: the dark layer standing in for the screen brightness. */
  const [shade, setShade] = useState(1)

  useLayoutEffect(() => {
    const el = box.current
    if (!el) return
    const measure = () => setSize({ w: el.clientWidth, h: el.clientHeight })
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  useEffect(() => {
    if (!ripple) return
    const timer = window.setTimeout(() => setRipple(null), RIPPLE_MS)
    return () => window.clearTimeout(timer)
  }, [ripple])

  // leaving the player gives the screen brightness back to the system
  useEffect(() => () => writeScreenBrightness(-1), [])

  // ---- gestures on the picture ----
  const gesture = useRef<Gesture | null>(null)
  const longTimer = useRef<number | undefined>(undefined)
  const tapTimer = useRef<number | undefined>(undefined)
  const hudTimer = useRef<number | undefined>(undefined)
  const lastTap = useRef<{ side: Side; at: number } | null>(null)
  useEffect(
    () => () => {
      window.clearTimeout(longTimer.current)
      window.clearTimeout(tapTimer.current)
      window.clearTimeout(hudTimer.current)
    },
    [],
  )
  const live = useRef(p)
  live.current = p

  const showHud = (h: { text: string; label?: string; level?: number }, ms?: number) => {
    window.clearTimeout(hudTimer.current)
    setHud(h)
    if (ms) hudTimer.current = window.setTimeout(() => setHud(null), ms)
  }

  const onTap = (side: Side, now: number) => {
    const last = lastTap.current
    if (last && last.side === side && now - last.at < TAP_MS) {
      // second and every further quick tap: seek; the bars stay as they were
      window.clearTimeout(tapTimer.current)
      tapTimer.current = undefined
      const step = side === 'left' ? -SEEK_BACK : SEEK_FWD
      p.seekTo(p.nowAt() + step)
      setRipple((r) => ({ side, total: (r && r.side === side ? r.total : 0) + Math.abs(step), key: now }))
      if (p.shown) p.show()
      lastTap.current = { side, at: now }
      return
    }
    lastTap.current = { side, at: now }
    window.clearTimeout(tapTimer.current)
    tapTimer.current = window.setTimeout(() => {
      tapTimer.current = undefined
      const q = live.current
      if (q.shown) q.hide()
      else q.show()
    }, TAP_MS)
  }

  const onDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (!e.isPrimary || (e.pointerType === 'mouse' && e.button !== 0)) return
    try {
      e.currentTarget.setPointerCapture(e.pointerId)
    } catch {
      // synthetic or already gone pointer
    }
    const r = e.currentTarget.getBoundingClientRect()
    const g: Gesture = {
      id: e.pointerId,
      x: e.clientX,
      y: e.clientY,
      side: e.clientX - r.left < r.width / 2 ? 'left' : 'right',
      moved: false,
      mode: 'pending',
      from: 0,
      height: r.height,
      rate: 1,
    }
    gesture.current = g
    window.clearTimeout(longTimer.current)
    if (p.playing) {
      longTimer.current = window.setTimeout(() => {
        const el = live.current.media()
        if (gesture.current !== g || g.moved || g.mode !== 'pending' || !el) return
        g.mode = 'long'
        g.rate = el.playbackRate
        el.playbackRate = LONG_PRESS_RATE
        showHud({ text: `${LONG_PRESS_RATE}x` })
      }, LONG_PRESS_MS)
    }
  }

  const onMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const g = gesture.current
    if (!g || g.id !== e.pointerId || g.mode === 'long') return
    const dx = e.clientX - g.x
    const dy = e.clientY - g.y
    if (g.mode === 'pending') {
      if (!g.moved && Math.hypot(dx, dy) > SLOP) {
        g.moved = true
        window.clearTimeout(longTimer.current)
      }
      if (Math.abs(dy) < SWIPE_MIN || Math.abs(dy) < SWIPE_RATIO * Math.abs(dx)) return
      g.mode = 'swipe'
      g.kind = g.side === 'left' ? 'brightness' : 'volume'
      g.from = g.kind === 'volume' ? p.volume : hasScreenBrightness() ? readScreenBrightness() : shade
    }
    const v = clamp01(g.from - dy / (g.height * SWIPE_RANGE))
    if (g.kind === 'volume') p.setVolume(Math.round(v * 100) / 100)
    else if (hasScreenBrightness()) writeScreenBrightness(v)
    else setShade(v)
    showHud({ text: `${Math.round(v * 100)} %`, label: g.kind === 'volume' ? t('vp.volume') : t('m.brightness'), level: v })
  }

  const end = (e: ReactPointerEvent<HTMLDivElement>, cancelled: boolean) => {
    const g = gesture.current
    if (!g || g.id !== e.pointerId) return
    gesture.current = null
    window.clearTimeout(longTimer.current)
    if (g.mode === 'long') {
      const el = p.media()
      if (el) el.playbackRate = g.rate
      window.clearTimeout(hudTimer.current)
      setHud(null)
      return
    }
    if (g.mode === 'swipe') {
      window.clearTimeout(hudTimer.current)
      hudTimer.current = window.setTimeout(() => setHud(null), HUD_MS)
      return
    }
    if (!cancelled && !g.moved) onTap(g.side, e.timeStamp)
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
    try {
      e.currentTarget.setPointerCapture(e.pointerId)
    } catch {
      // synthetic pointer
    }
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

  // ---- sizes ----
  const s = controlSizes(size.w, size.h)
  const wide = size.w >= WIDE_MIN_WIDTH
  const topCount = 2 + (p.quality ? 1 : 0) + (p.subs ? 1 : 0) + (p.audio ? 1 : 0) + 1
  const barButton = fitBarButton(s.button, wide ? topCount + 3 : topCount, size.w - 2 * s.padX)
  const vars = {
    '--vt-btn': `${barButton}px`,
    '--vt-play': `${s.play}px`,
    '--vt-seek': `${s.seek}px`,
    '--vt-gap': `${s.centerGap}px`,
    '--vt-px': `${s.padX}px`,
    '--vt-pt': `${s.padTop}px`,
    '--vt-pb': `${s.padBottom}px`,
    '--vt-time': `${s.timeText}px`,
    '--vt-title': `${s.titleText}px`,
    '--vt-track': `${s.track}px`,
    '--vt-thumb': `${s.thumb}px`,
  } as CSSProperties

  const shownTime = p.scrub ?? p.time
  const pct = (sec: number) => (p.duration > 0 ? Math.min(100, Math.max(0, (sec / p.duration) * 100)) : 0)
  const keep = () => p.show()
  const close = () => {
    if (p.fullscreen) p.toggleFullscreen()
    // came here from inside Flimmer (the router counts its entries): back to that page, else home
    else if (((window.history.state as { idx?: number } | null)?.idx ?? 0) > 0) window.history.back()
    else window.location.assign(p.backTo ?? '/')
  }

  // the sheet keeps its last content while it slides out
  const open = p.menu
  const lastMenu = useRef(open)
  if (open) lastMenu.current = open
  const sheetMenu = open ?? lastMenu.current
  const groups: Pick[] = sheetMenu && sheetMenu !== 'settings' && p[sheetMenu] ? [p[sheetMenu] as Pick] : []
  const sheetTitle = groups[0]?.title ?? ''

  return (
    <div
      ref={box}
      className={`vt${p.shown ? ' is-shown' : ''}${wide ? ' is-wide' : ''}${p.fullscreen ? ' is-fullscreen' : ''}${p.scrub !== null ? ' is-scrubbing' : ''}`}
      style={vars}
    >
      {shade < 1 && <div className="vt-shade" style={{ opacity: (1 - shade) * 0.85 }} aria-hidden="true" />}
      <div
        className="vt-surface"
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={(e) => end(e, false)}
        onPointerCancel={(e) => end(e, true)}
        onContextMenu={(e) => e.preventDefault()}
        aria-hidden="true"
      />

      {ripple && (
        <div className={`vt-ripple is-${ripple.side}`} key={ripple.key} aria-live="polite">
          <Icon name={ripple.side === 'left' ? 'back10' : 'fwd10'} size={22} />
          {t(ripple.side === 'left' ? 'm.seekBack' : 'm.seekFwd', { n: ripple.total })}
        </div>
      )}
      {hud && (
        <div className="vt-hud" role="status">
          {hud.label && <span className="vt-hud-label">{hud.label}</span>}
          <span className="vt-hud-text">{hud.text}</span>
          {hud.level !== undefined && (
            <span className="vt-hud-meter" aria-hidden="true">
              <span style={{ transform: `scaleX(${hud.level})` }} />
            </span>
          )}
        </div>
      )}

      <div className="vt-chrome">
        <div className="vt-top" onPointerDownCapture={keep}>
          <button type="button" className="vt-btn" onClick={close} aria-label={t('m.back')}>
            <Icon name="left" size={s.icon} />
          </button>
          <div className="vt-title">
            {wide && <p className="vt-title-main">{p.title}</p>}
            {wide && p.subtitle && <p className="vt-title-sub">{p.subtitle}</p>}
          </div>
          {(p.audio || p.subs) && (
            <button type="button" className="vt-btn" onClick={() => p.setMenu('settings')} aria-label={t('settings.title')} aria-haspopup="dialog">
              <Icon name="gear" size={s.icon} />
            </button>
          )}
          {p.quality && (
            <button type="button" className="vt-btn" onClick={() => p.setMenu('quality')} aria-label={p.quality.title} aria-haspopup="dialog">
              <Icon name="layers" size={s.icon} />
            </button>
          )}
          {p.subs && (
            <button type="button" className="vt-btn" onClick={() => p.setMenu('subs')} aria-label={p.subs.title} aria-haspopup="dialog">
              <Icon name="subtitles" size={s.icon} />
            </button>
          )}
          {p.audio && (
            <button type="button" className="vt-btn" onClick={() => p.setMenu('audio')} aria-label={p.audio.title} aria-haspopup="dialog">
              <Icon name="audio" size={s.icon} />
            </button>
          )}
          <button
            type="button"
            className="vt-btn"
            onClick={p.toggleMute}
            aria-label={p.muted ? t('vp.unmute') : t('vp.mute')}
            aria-pressed={p.muted}
          >
            <Icon name={p.muted ? 'mute' : 'volume'} size={s.icon} />
          </button>
        </div>

        <div className="vt-mid">
          {p.scrub !== null ? (
            <p className="vt-scrub-time" aria-hidden="true">
              {formatClock(p.scrub)}
            </p>
          ) : (
            <div className="vt-mid-row" onPointerDownCapture={keep}>
              <button type="button" className="vt-seek" onClick={() => p.seekTo(p.nowAt() - SEEK_BACK)} aria-label={t('m.seekBackLabel')}>
                <Icon name="back10" size={Math.round(s.seek * 0.46)} />
                <span aria-hidden="true">{SEEK_BACK}</span>
              </button>
              {p.busy ? (
                <span className="vt-play is-spacer" aria-hidden="true" />
              ) : (
                <button type="button" className="vt-play" onClick={p.toggle} aria-label={p.playing ? t('vp.pause') : t('common.play')}>
                  <Icon name={p.playing ? 'pause' : 'play'} size={s.playIcon} />
                </button>
              )}
              <button type="button" className="vt-seek" onClick={() => p.seekTo(p.nowAt() + SEEK_FWD)} aria-label={t('m.seekFwdLabel')}>
                <Icon name="fwd10" size={Math.round(s.seek * 0.46)} />
                <span aria-hidden="true">{SEEK_FWD}</span>
              </button>
            </div>
          )}
        </div>

        <div className="vt-bottom" onPointerDownCapture={keep}>
          <div className="vt-row">
            <button type="button" className="vt-time" onClick={() => setRemaining((r) => !r)} aria-label={t('m.timeToggle')}>
              {remaining ? `-${formatClock(p.duration - shownTime)}` : formatClock(shownTime)}
              <span className="vt-time-total"> / {formatClock(p.duration)}</span>
            </button>
            {p.status && <span className="vt-status">{p.status}</span>}
            <span className="vt-spacer" />
            {p.next && (
              <button type="button" className="vt-next" onClick={p.next.go} aria-label={t('vp.nextLabel', { label: p.next.label })}>
                <Icon name="next" size={Math.max(16, s.icon - 4)} />
                {t('player.nextEpisode')}
              </button>
            )}
            <button
              type="button"
              className="vt-btn"
              onClick={p.toggleFullscreen}
              aria-label={p.fullscreen ? t('vp.exitFullscreen') : t('vp.fullscreen')}
            >
              <Icon name={p.fullscreen ? 'shrink' : 'expand'} size={s.icon} />
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
          </div>
        </div>
      </div>

      <Sheet open={!!open && open !== 'settings'} onClose={() => p.setMenu(null)} title={sheetTitle} inline={p.fullscreen} className="vt-sheet">
        {groups.map((g) => (
          <div key={g.title} role="menu" aria-label={g.title}>
            {g.options.map((o) => (
              <SheetOption
                key={o.id}
                checked={o.id === g.value}
                label={o.label}
                hint={o.hint}
                onPick={() => {
                  g.onPick(o.id)
                  p.setMenu(null)
                }}
              />
            ))}
          </div>
        ))}
      </Sheet>
      <SettingsMenu
        t={t}
        open={open === 'settings'}
        onClose={() => p.setMenu(null)}
        panel={size.w > size.h}
        inline={p.fullscreen}
        media={p.media}
        audio={p.audio}
        subs={p.subs}
        hasSubs={p.hasSubs}
        subOn={p.subOn}
        cancelNext={p.cancelNext}
      />
    </div>
  )
}
