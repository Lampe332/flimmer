import { useRef, type ReactNode } from 'react'
import Icon from './Icon'

type Props = { title: string; children: ReactNode; action?: ReactNode }

/** Horizontal scroll shelf with paging buttons (pointer devices) and native scroll/snap. */
export default function Row({ title, children, action }: Props) {
  const track = useRef<HTMLDivElement>(null)

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
          <button type="button" className="icon-btn" onClick={() => page(-1)} aria-label={`${title}: zurückblättern`}>
            <Icon name="left" />
          </button>
          <button type="button" className="icon-btn" onClick={() => page(1)} aria-label={`${title}: weiterblättern`}>
            <Icon name="right" />
          </button>
        </div>
      </div>
      <div className="row-track" ref={track}>
        {children}
      </div>
    </section>
  )
}
