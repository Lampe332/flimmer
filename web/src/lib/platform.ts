import { useSyncExternalStore } from 'react'

/**
 * Phone layout: the same width rule as the stylesheet's phone block (max-width: 760px). Touch devices with a mouse
 * on a wide screen keep the desktop layout; a desktop browser made narrow gets the phone layout, like the CSS.
 */
export const PHONE_MQ = '(max-width: 760px)'
/** A finger is the main pointer (phones, tablets): the player gets its touch controls. */
export const COARSE_MQ = '(pointer: coarse)'

/** The Android app (Android System WebView, "; wv)" in the user agent). */
export const IS_ANDROID_APP = /Android/.test(navigator.userAgent) && /; wv\)/.test(navigator.userAgent)

function useMedia(query: string): boolean {
  return useSyncExternalStore(
    (on) => {
      const mq = window.matchMedia(query)
      mq.addEventListener('change', on)
      return () => mq.removeEventListener('change', on)
    },
    () => window.matchMedia(query).matches,
  )
}

export const useIsPhone = () => useMedia(PHONE_MQ)
/**
 * The player page without app chrome: phones upright (the phone layout) and phones turned sideways (wide but low).
 * Same rule as the `.is-bare` player styles in mobile.css.
 */
export const PHONE_PLAYER_MQ = `${PHONE_MQ}, (orientation: landscape) and (max-height: 500px)`
export const usePhonePlayer = () => useMedia(PHONE_PLAYER_MQ)
export const useCoarse = () => useMedia(COARSE_MQ)
export const isPhone = () => window.matchMedia(PHONE_MQ).matches

/**
 * TMDB artwork in the size the phone needs: a 104 px poster is drawn from w342 instead of w500 and a 250 px wide
 * card from w780 instead of w1280 (less than half the pixels to load and decode). Desktop keeps the server's sizes.
 */
export function phoneImage(url: string | null | undefined, kind: 'poster' | 'wide'): string | undefined {
  if (!url) return undefined
  if (!isPhone()) return url
  return kind === 'poster' ? url.replace('/t/p/w500/', '/t/p/w342/') : url.replace('/t/p/w1280/', '/t/p/w780/')
}

// ---------------------------------------------------------------- Back closes the top sheet first

type Closer = () => void
const stack: Closer[] = []

/**
 * Android Back while a sheet is open closes the sheet. Outside fullscreen the sheet adds a history entry that Back
 * pops (the WebView goes back in its history); in fullscreen the app asks `window.flimmerBack()` before it leaves
 * fullscreen (MainActivity). Returns a function that closes from the UI and removes the entry again.
 */
export function pushBack(close: Closer): () => void {
  stack.push(close)
  const marker = { ...(window.history.state ?? {}), flimmerSheet: stack.length }
  window.history.pushState(marker, '')
  let done = false
  const onPop = () => {
    if (done) return
    done = true
    window.removeEventListener('popstate', onPop)
    remove()
    close()
  }
  const remove = () => {
    const i = stack.lastIndexOf(close)
    if (i >= 0) stack.splice(i, 1)
  }
  window.addEventListener('popstate', onPop)
  return () => {
    if (done) return
    done = true
    window.removeEventListener('popstate', onPop)
    remove()
    if (window.history.state?.flimmerSheet) window.history.back()
  }
}

declare global {
  interface Window {
    flimmerBack?: () => boolean
  }
}
/** Called by the Android app on Back in fullscreen: true when a sheet was open and is now closed. */
window.flimmerBack = () => {
  const top = stack.at(-1)
  if (!top) return false
  window.history.back()
  return true
}
