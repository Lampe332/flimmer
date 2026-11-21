/**
 * Black bars burned into the picture: many films come as a 16:9 frame with the cinema bars baked in. Then
 * "Einpassen" fits the whole frame to the screen height and the bars show on all four sides. Every few seconds a
 * small copy of the frame is checked for black rows and columns at its edges; the bars found are cut off with
 * `object-view-box`, so the picture itself fits (or with "Zoomen" fills) the screen. Bars only ever get smaller
 * while a source plays, so a dark scene cannot cut into the picture for long; a new source starts over.
 */
import { useEffect } from 'react'

const SAMPLE_W = 160
const DARK = 34 // brightest a bar pixel may be (0 to 255, video black is about 16)
const MIN_BAR = 0.02 // thinner bars (share of the side) are left alone
const FIRST_AFTER = 3 // samples needed before the first cut
const MAX_BAR = 0.22 // more is no bar but a dark scene or a logo on black
const supported = typeof CSS !== 'undefined' && CSS.supports('object-view-box', 'inset(1% 0% 1% 0%)')

/** Dark lines from both edges (the thinner side), as a share of the axis; -1 when the whole frame is dark. */
function bars(px: Uint8ClampedArray, w: number, h: number, rows: boolean): number {
  const lines = rows ? h : w
  const len = rows ? w : h
  const dark = (i: number) => {
    let bright = 0
    for (let j = 0; j < len; j++) {
      const o = ((rows ? i * w + j : j * w + i) * 4) | 0
      if (0.299 * px[o]! + 0.587 * px[o + 1]! + 0.114 * px[o + 2]! > DARK && ++bright > len * 0.02) return false
    }
    return true
  }
  let a = 0
  while (a < lines / 2 && dark(a)) a++
  if (a >= lines / 2) return -1
  let b = 0
  while (b < lines / 2 && dark(lines - 1 - b)) b++
  const n = Math.min(a, b)
  // the first bright line is mostly a blend of bar and picture: cut half of it, else a hairline of black stays
  return n ? (n + 0.5) / lines : 0
}

/** `media` is read once: the player keeps one video element and changes only its source. */
export function useLetterbox(media: () => HTMLVideoElement | null) {
  useEffect(() => {
    const el = media()
    if (!el || !supported) return
    const canvas = document.createElement('canvas')
    const ctx = canvas.getContext('2d', { willReadFrequently: true })
    if (!ctx) return
    let cut = { y: 1, x: 1 }
    let seen = 0
    let broken = false
    const apply = () => {
      const y = seen >= FIRST_AFTER && cut.y >= MIN_BAR ? cut.y : 0
      const x = seen >= FIRST_AFTER && cut.x >= MIN_BAR ? cut.x : 0
      el.style.setProperty('object-view-box', y || x ? `inset(${y * 100}% ${x * 100}% ${y * 100}% ${x * 100}%)` : '')
    }
    const reset = () => {
      cut = { y: 1, x: 1 }
      seen = 0
      apply()
    }
    const sample = () => {
      if (broken || el.readyState < 2 || !el.videoWidth || !el.videoHeight) return
      const w = SAMPLE_W
      const h = Math.max(8, Math.round((SAMPLE_W * el.videoHeight) / el.videoWidth))
      canvas.width = w
      canvas.height = h
      let px: Uint8ClampedArray
      try {
        ctx.drawImage(el, 0, 0, w, h)
        px = ctx.getImageData(0, 0, w, h).data
      } catch {
        broken = true // a cross-origin frame cannot be read: keep the picture as it is
        return
      }
      const y = bars(px, w, h, true)
      const x = bars(px, w, h, false)
      if (y < 0 || x < 0 || y > MAX_BAR || x > MAX_BAR) return // dark frame (fade, logo, credits): says nothing
      cut = { y: Math.min(cut.y, y), x: Math.min(cut.x, x) }
      seen++
      apply()
    }
    const timer = window.setInterval(() => {
      if (!el.paused || seen < FIRST_AFTER) sample()
    }, 1500)
    el.addEventListener('loadstart', reset)
    el.addEventListener('seeked', sample)
    return () => {
      window.clearInterval(timer)
      el.removeEventListener('loadstart', reset)
      el.removeEventListener('seeked', sample)
      el.style.removeProperty('object-view-box')
    }
  }, [])
}
