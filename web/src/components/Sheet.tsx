import { useEffect, useId, useRef, useState, type ReactNode, type PointerEvent as ReactPointerEvent } from 'react'
import { createPortal } from 'react-dom'
import { pushBack } from '../lib/platform'

type Props = {
  open: boolean
  onClose: () => void
  title: string
  children: ReactNode
  /** Render in place instead of on <body>: the player's sheets must stay inside its fullscreen box. */
  inline?: boolean
  className?: string
}

/**
 * Bottom sheet for phones: slides up from the bottom edge, a drag on the handle or the title pulls it down, a tap on
 * the shade, Escape and Android Back close it. Only transform and opacity animate.
 */
export default function Sheet({ open, onClose, title, children, inline, className }: Props) {
  const [shown, setShown] = useState(open)
  const [leaving, setLeaving] = useState(false)
  const panel = useRef<HTMLDivElement>(null)
  const id = useId()
  const closeRef = useRef(onClose)
  closeRef.current = onClose
  const drag = useRef<{ y: number; dy: number; t: number } | null>(null)

  // mount on open, play the exit before unmounting
  useEffect(() => {
    if (open) {
      setShown(true)
      setLeaving(false)
      return
    }
    if (!shown) return
    setLeaving(true)
    const t = window.setTimeout(() => (setShown(false), setLeaving(false)), 200)
    return () => window.clearTimeout(t)
  }, [open, shown])

  // Back closes; closing from the UI drops the history entry again
  useEffect(() => {
    if (!open) return
    const release = pushBack(() => closeRef.current())
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        closeRef.current()
      }
    }
    window.addEventListener('keydown', onKey, true)
    const before = document.activeElement as HTMLElement | null
    requestAnimationFrame(() => panel.current?.querySelector<HTMLElement>('[aria-checked="true"], button, a, input')?.focus({ preventScroll: true }))
    return () => {
      window.removeEventListener('keydown', onKey, true)
      release()
      before?.focus?.({ preventScroll: true })
    }
  }, [open])

  if (!shown) return null

  const onDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    drag.current = { y: e.clientY, dy: 0, t: e.timeStamp }
    e.currentTarget.setPointerCapture(e.pointerId)
  }
  const onMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current
    if (!d || !panel.current) return
    d.dy = Math.max(0, e.clientY - d.y)
    panel.current.style.transition = 'none'
    panel.current.style.transform = `translateY(${d.dy}px)`
  }
  const onUp = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current
    drag.current = null
    const el = panel.current
    if (!d || !el) return
    el.style.transition = ''
    el.style.transform = ''
    const fast = d.dy / Math.max(1, e.timeStamp - d.t) > 0.6
    if (d.dy > el.offsetHeight * 0.3 || (fast && d.dy > 24)) onClose()
  }

  const sheet = (
    <div className={`sheet${leaving ? ' is-leaving' : ''}${inline ? ' is-inline' : ''}${className ? ` ${className}` : ''}`}>
      <div className="sheet-shade" onClick={onClose} aria-hidden="true" />
      <div className="sheet-panel" role="dialog" aria-modal="true" aria-labelledby={id} ref={panel}>
        <div className="sheet-grip" onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp} onPointerCancel={onUp}>
          <span className="sheet-handle" aria-hidden="true" />
          <h2 className="sheet-title" id={id}>
            {title}
          </h2>
        </div>
        <div className="sheet-body">{children}</div>
      </div>
    </div>
  )
  return inline ? sheet : createPortal(sheet, document.body)
}

/** One choice in a sheet: a full-width row with a check on the current one. */
export function SheetOption({
  checked,
  label,
  hint,
  onPick,
  lang,
}: {
  checked: boolean
  label: string
  hint?: string
  onPick: () => void
  lang?: string
}) {
  return (
    <button type="button" role="menuitemradio" aria-checked={checked} className="sheet-option" onClick={onPick} lang={lang}>
      <span className="sheet-option-label">{label}</span>
      {hint && <span className="sheet-option-hint">{hint}</span>}
      <span className="sheet-option-check" aria-hidden="true" />
    </button>
  )
}
