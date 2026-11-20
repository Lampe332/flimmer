import { useEffect, useRef, useState, type ReactNode } from 'react'
import Icon from './Icon'

type Props = { title: string; children: ReactNode; action?: ReactNode }

/** Horizontal scroll shelf: native scroll/snap, paging buttons that know when an end is reached. */
export default function Row({ title, children, action }: Props) {
  const track = useRef<HTMLDivElement>(null)
  const [edge, setEdge] = useState({ start: true, end: false })

  useEffect(() => {
    const el = track.current
    if (!el) return
    const update = () => {
      const start = el.scrollLeft <= 4
      const end = el.scrollLeft + el.clientWidth >= el.scrollWidth - 4
      setEdge((prev) => (prev.start === start && prev.end === end ? prev : { start, end }))
    }
    update()
    el.addEventListener('scroll', update, { passive: true })
    const ro = new ResizeObserver(update)
    ro.observe(el)
    return () => {
      el.removeEventListener('scroll', update)
      ro.disconnect()
    }
  }, [children])

  const page = (dir: 1 | -1) => {
    const el = track.current
    if (!el) return
    el.scrollBy({ left: dir * el.clientWidth * 0.85, behavior: 'smooth' })
  }

  return (
    <section className="row" aria-label={title}>
      <div className="row-head">
        <h2 className="row-title">{title}</h2>
        {action}
        <div className="row-nav">
          <button
            type="button"
            className="icon-btn"
            onClick={() => page(-1)}
            disabled={edge.start}
            aria-label={`${title}: zurückblättern`}
          >
            <Icon name="left" size={18} />
          </button>
          <button
            type="button"
            className="icon-btn"
            onClick={() => page(1)}
            disabled={edge.end}
            aria-label={`${title}: weiterblättern`}
          >
            <Icon name="right" size={18} />
          </button>
        </div>
      </div>
      <div className={`row-track${edge.start ? '' : ' fade-start'}${edge.end ? '' : ' fade-end'}`} ref={track}>
        {children}
      </div>
    </section>
  )
}
