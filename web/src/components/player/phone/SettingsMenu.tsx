/**
 * The phone player's settings menu: speed, picture fit, sleep timer, subtitle delay and style, then the audio and
 * subtitle tracks. Upright a bottom sheet, sideways a panel under the top bar; sub pages have a back arrow and Back.
 *
 * Menu layout and rules taken from Seanime Mobile (github.com/Lampe332/seanime-mobile, GPL-3.0), files
 * native/src/main/_features/player/player-menu.tsx (main list, sub pages) and player-menu-logic.ts (speed range,
 * steps and presets, delay steps). Adapted to the web: HTML video, TextTrack cues, Flimmer's Sheet and prefs.
 */
import { useEffect, useRef, useState } from 'react'
import type { T } from '../../../i18n'
import { formatMs } from '../../../lib/format'
import { pushBack } from '../../../lib/platform'
import { SUB_BGS, SUB_SIZES, VIDEO_FITS, setPrefs, usePrefs, type SubBg, type SubSize, type VideoFit } from '../../../lib/prefs'
import Icon from '../../Icon'
import Sheet, { SheetOption } from '../../Sheet'

export const SPEED_MIN = 0.25
export const SPEED_MAX = 2
export const SPEED_STEP = 0.05
export const SPEED_PRESETS = [0.5, 1, 1.2, 1.5, 2]
export const DELAY_STEP = 0.1
/** Sleep timer in minutes; -1 = at the end of the episode. */
export const SLEEP_OPTIONS = [0, 15, 30, 45, 60, -1]

const roundTo = (v: number, step: number) => Math.round(v / step) * step
export const clampSpeed = (v: number) => Math.min(SPEED_MAX, Math.max(SPEED_MIN, Math.round(roundTo(v, SPEED_STEP) * 100) / 100))
export const speedLabel = (v: number) => `${v.toFixed(2)}x`

type Group = { title: string; value: number; options: { id: number; label: string; hint?: string }[]; onPick: (id: number) => void }
type Page = 'speed' | 'fit' | 'sleep' | 'delay' | 'style'

type Props = {
  t: T
  open: boolean
  onClose: () => void
  /** Sideways: a panel under the top bar instead of the bottom sheet. */
  panel: boolean
  inline: boolean
  media: () => HTMLVideoElement | null
  audio: Group | null
  subs: Group | null
  /** The file has text subtitles / one of them is on. */
  hasSubs: boolean
  subOn: boolean
  /** Stops the next episode from starting by itself (sleep timer "end of episode"). */
  cancelNext?: () => void
}

const FIT_KEY = { contain: 'm.fitContain', cover: 'm.fitCover', fill: 'm.fitFill' } as const
const SIZE_KEY = { s: 'm.sizeS', m: 'm.sizeM', l: 'm.sizeL' } as const
const BG_KEY = { none: 'm.bgNone', dim: 'm.bgDim', solid: 'm.bgSolid' } as const

export default function SettingsMenu(p: Props) {
  const { t } = p
  const prefs = usePrefs()
  const [page, setPage] = useState<Page | null>(null)
  /** Speed is per playback: every new player starts at 1x. */
  const [speed, setSpeed] = useState(1)
  const [delay, setDelay] = useState(0)
  const [sleep, setSleep] = useState(0)
  const live = useRef(p)
  live.current = p

  // a sub page is a step Back closes before the menu itself
  useEffect(() => {
    if (!p.open || !page) return
    return pushBack(() => setPage(null))
  }, [p.open, page])

  const close = () => {
    setPage(null)
    p.onClose()
  }

  // speed (the default rate survives a new source)
  useEffect(() => {
    const el = live.current.media()
    if (!el) return
    el.defaultPlaybackRate = speed
    el.playbackRate = speed
  }, [speed])

  // picture fit and subtitle style live on the video element, only while the phone controls are on
  useEffect(() => {
    const el = live.current.media()
    if (!el) return
    el.style.objectFit = prefs.videoFit
    el.dataset.subSize = prefs.subSize
    el.dataset.subBg = prefs.subBg
    return () => {
      el.style.objectFit = ''
      delete el.dataset.subSize
      delete el.dataset.subBg
    }
  }, [prefs.videoFit, prefs.subSize, prefs.subBg])

  // subtitle delay: every cue of the shown tracks moves by `delay` from its own time (new cues as they arrive)
  const origin = useRef(new WeakMap<TextTrackCue, [number, number]>())
  useEffect(() => {
    const el = live.current.media()
    if (!el) return
    const apply = () => {
      for (const track of Array.from(el.textTracks)) {
        if (track.mode === 'disabled' || !track.cues) continue
        for (const cue of Array.from(track.cues)) {
          let o = origin.current.get(cue)
          if (!o) origin.current.set(cue, (o = [cue.startTime, cue.endTime]))
          const start = Math.max(0, o[0] + delay)
          if (cue.startTime !== start) {
            cue.startTime = start
            cue.endTime = Math.max(start, o[1] + delay)
          }
        }
      }
    }
    apply()
    el.addEventListener('timeupdate', apply)
    el.textTracks.addEventListener('change', apply)
    return () => {
      el.removeEventListener('timeupdate', apply)
      el.textTracks.removeEventListener('change', apply)
    }
  }, [delay])

  // sleep timer: pause when it runs out; "end of episode" lets the episode end without the next one starting
  useEffect(() => {
    if (sleep > 0) {
      const timer = window.setTimeout(() => {
        live.current.media()?.pause()
        setSleep(0)
      }, sleep * 60_000)
      return () => window.clearTimeout(timer)
    }
    if (sleep < 0) {
      const el = live.current.media()
      if (!el) return
      const check = () => {
        if (el.duration > 0 && el.duration - el.currentTime <= 6) live.current.cancelNext?.()
      }
      const done = () => setSleep(0)
      el.addEventListener('timeupdate', check)
      el.addEventListener('ended', done)
      return () => {
        el.removeEventListener('timeupdate', check)
        el.removeEventListener('ended', done)
      }
    }
  }, [sleep])

  const sleepLabel = (m: number) => (m === 0 ? t('vp.off') : m < 0 ? t('m.sleepEnd') : t('m.sleepMin', { n: m }))
  const delayLabel = (d: number) => `${d > 0 ? '+' : ''}${formatMs(Math.round(d * 10) * 100)}`
  const pageTitle: Record<Page, string> = {
    speed: t('m.speed'),
    fit: t('m.fit'),
    sleep: t('m.sleep'),
    delay: t('m.subDelay'),
    style: t('m.subStyle'),
  }

  const row = (id: Page, label: string, value: string) => (
    <button type="button" className="vt-menu-row" onClick={() => setPage(id)} aria-haspopup="true">
      <span className="vt-menu-row-label">{label}</span>
      <span className="vt-menu-row-value">{value}</span>
      <Icon name="right" size={18} />
    </button>
  )
  const group = (g: Group) => (
    <div className="vt-menu-group" role="menu" aria-label={g.title} key={g.title}>
      <p className="vt-sheet-head">{g.title}</p>
      {g.options.map((o) => (
        <SheetOption key={o.id} checked={o.id === g.value} label={o.label} hint={o.hint} onPick={() => g.onPick(o.id)} />
      ))}
    </div>
  )

  let body
  if (!page) {
    body = (
      <div className="vt-menu-main">
        <div className="vt-menu-group">
          {row('speed', t('m.speed'), speedLabel(speed))}
          {row('fit', t('m.fit'), t(FIT_KEY[prefs.videoFit]))}
          {row('sleep', t('m.sleep'), sleepLabel(sleep))}
          {p.subOn && row('delay', t('m.subDelay'), delayLabel(delay))}
          {p.hasSubs && row('style', t('m.subStyle'), `${t(SIZE_KEY[prefs.subSize])}, ${t(BG_KEY[prefs.subBg])}`)}
        </div>
        {(p.audio || p.subs) && (
          <div className="vt-menu-tracks">
            {p.audio && group({ ...p.audio, title: t('m.sound') })}
            {p.subs && group(p.subs)}
          </div>
        )}
      </div>
    )
  } else {
    const options = <V extends string | number>(values: readonly V[], current: V, label: (v: V) => string, pick: (v: V) => void) =>
      values.map((v) => <SheetOption key={String(v)} checked={v === current} label={label(v)} onPick={() => pick(v)} />)
    body = (
      <div className={`vt-menu-page is-${page}`}>
        <button type="button" className="vt-menu-back" onClick={() => setPage(null)} aria-label={t('m.back')}>
          <Icon name="left" size={20} />
          <span>{pageTitle[page]}</span>
        </button>
        {page === 'speed' && (
          <div className="vt-menu-tune">
            <div className="vt-stepper">
              <button type="button" className="vt-step" onClick={() => setSpeed((s) => clampSpeed(s - SPEED_STEP))} aria-label={t('m.slower')}>
                −
              </button>
              <output className="vt-step-value" aria-live="polite">
                {speedLabel(speed)}
              </output>
              <button type="button" className="vt-step" onClick={() => setSpeed((s) => clampSpeed(s + SPEED_STEP))} aria-label={t('m.faster')}>
                +
              </button>
            </div>
            <input
              className="vt-range"
              type="range"
              min={SPEED_MIN}
              max={SPEED_MAX}
              step={SPEED_STEP}
              value={speed}
              onChange={(e) => setSpeed(clampSpeed(Number(e.target.value)))}
              aria-label={t('m.speed')}
            />
            <div className="vt-chips">
              {SPEED_PRESETS.map((v) => (
                <button key={v} type="button" className="vt-chip" aria-pressed={speed === v} onClick={() => setSpeed(v)}>
                  {`${v}x`}
                </button>
              ))}
            </div>
          </div>
        )}
        {page === 'fit' && options<VideoFit>(VIDEO_FITS, prefs.videoFit, (v) => t(FIT_KEY[v]), (v) => setPrefs({ videoFit: v }))}
        {page === 'sleep' && options(SLEEP_OPTIONS, sleep, sleepLabel, setSleep)}
        {page === 'delay' && (
          <div className="vt-menu-tune">
            <div className="vt-stepper">
              <button type="button" className="vt-step is-wide" onClick={() => setDelay((d) => Math.round((d - DELAY_STEP) * 10) / 10)}>
                {t('m.earlier')}
              </button>
              <output className="vt-step-value" aria-live="polite">
                {delayLabel(delay)}
              </output>
              <button type="button" className="vt-step is-wide" onClick={() => setDelay((d) => Math.round((d + DELAY_STEP) * 10) / 10)}>
                {t('m.later')}
              </button>
            </div>
            <div className="vt-chips">
              <button type="button" className="vt-chip" onClick={() => setDelay(0)} disabled={delay === 0}>
                {t('m.reset')}
              </button>
            </div>
          </div>
        )}
        {page === 'style' && (
          <>
            <p className="vt-sheet-head">{t('m.subSize')}</p>
            {options<SubSize>(SUB_SIZES, prefs.subSize, (v) => t(SIZE_KEY[v]), (v) => setPrefs({ subSize: v }))}
            <p className="vt-sheet-head">{t('m.subBg')}</p>
            {options<SubBg>(SUB_BGS, prefs.subBg, (v) => t(BG_KEY[v]), (v) => setPrefs({ subBg: v }))}
          </>
        )}
      </div>
    )
  }

  return (
    <Sheet
      open={p.open}
      onClose={close}
      title={t('settings.title')}
      inline={p.inline || p.panel}
      className={`vt-sheet vt-menu${p.panel ? ' is-panel' : ''}`}
    >
      {body}
    </Sheet>
  )
}
