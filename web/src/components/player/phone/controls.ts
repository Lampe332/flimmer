/**
 * Sizes, timings and gesture rules of the phone player.
 *
 * Taken from Seanime Mobile (github.com/Lampe332/seanime-mobile, GPL-3.0), files
 * native/src/main/_features/player/control-sizes.ts (controlSizes, fitBarButton, BOX_COMPACT_HEIGHT) and
 * the gesture constants of native/src/main/_features/player/player-logic.ts / player-screen.tsx.
 * Adapted to the web: CSS px instead of dp, no React Native imports.
 */

export type ControlSizes = {
  button: number
  icon: number
  play: number
  playIcon: number
  seek: number
  centerGap: number
  padX: number
  padTop: number
  /** under the bottom bar, before the safe area */
  padBottom: number
  timeText: number
  titleText: number
  /** height of the time bar's visible track */
  track: number
  /** the time bar's thumb (1.35x while scrubbing) */
  thumb: number
}

export const TABLET_MIN_WIDTH = 1000
/** The title shows from this width (phone landscape). */
export const WIDE_MIN_WIDTH = 560
/** Under this height the middle row gets smaller buttons. */
export const COMPACT_HEIGHT = 300
/** The smallest top bar button. */
export const MIN_BAR_BUTTON = 36

export function controlSizes(width: number, height: number): ControlSizes {
  let s: ControlSizes
  if (width >= TABLET_MIN_WIDTH) {
    s = { button: 48, icon: 24, play: 88, playIcon: 38, seek: 68, centerGap: 64, padX: 24, padTop: 14, padBottom: 8, timeText: 15, titleText: 15, track: 5, thumb: 16 }
  } else {
    const wide = width >= WIDE_MIN_WIDTH
    s = {
      button: 44,
      icon: wide ? 22 : 20,
      play: wide ? 76 : 60,
      playIcon: wide ? 34 : 27,
      seek: wide ? 60 : 48,
      centerGap: wide ? 40 : 20,
      padX: wide ? 14 : 6,
      padTop: wide ? 10 : 4,
      padBottom: wide ? 6 : 2,
      timeText: wide ? 14 : 12,
      titleText: 14,
      track: wide ? 4 : 3,
      thumb: wide ? 14 : 12,
    }
  }
  if (height > 0 && height < COMPACT_HEIGHT) return { ...s, play: 48, playIcon: 22, seek: 40, centerGap: 16 }
  return s
}

/** Top bar buttons get narrower when the row is full, never under MIN_BAR_BUTTON. `room` = the row's inner width. */
export function fitBarButton(size: number, count: number, room: number): number {
  if (count <= 0) return size
  return Math.min(size, Math.max(MIN_BAR_BUTTON, Math.floor(room / count)))
}

/** The bars hide this long after the last touch while playing. */
export const HIDE_MS = 5000
/** A single tap waits this long for a second one (double tap). */
export const TAP_MS = 280
export const SEEK_BACK = 5
export const SEEK_FWD = 10
export const RIPPLE_MS = 700
export const LONG_PRESS_MS = 450
export const LONG_PRESS_RATE = 2
/** Moving further than this cancels a tap or a long press. */
export const SLOP = 10
/** A vertical swipe starts after this many px, mostly vertical. */
export const SWIPE_MIN = 24
export const SWIPE_RATIO = 1.5
/** A swipe over this share of the height changes the value from 0 to 1. */
export const SWIPE_RANGE = 0.75
export const HUD_MS = 600

export const clamp01 = (v: number) => Math.min(1, Math.max(0, v))

// ---- screen brightness: the Android app's bridge (MainActivity), else a dark layer over the picture

type ScreenBridge = { getBrightness(): number; setBrightness(value: number): void }
const bridge = (): ScreenBridge | undefined => (window as Window & { FlimmerScreen?: ScreenBridge }).FlimmerScreen

/** True when the app can set the real screen brightness. */
export const hasScreenBrightness = () => !!bridge()

export function readScreenBrightness(): number {
  try {
    return clamp01(bridge()?.getBrightness() ?? 1)
  } catch {
    return 1
  }
}

/** A value from 0 to 1 sets the screen; a negative one gives the brightness back to the system. */
export function writeScreenBrightness(value: number) {
  try {
    bridge()?.setBrightness(value)
  } catch {
    // the bridge is gone (page left): nothing to set
  }
}
