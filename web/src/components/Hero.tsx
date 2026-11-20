import { useState } from 'react'
import { Link } from 'react-router'
import { playStartPath, titlePath, type Item } from '../lib/api'
import { formatRating, typeLabel } from '../lib/format'
import Icon from './Icon'
import SaveButton from './SaveButton'

const prefersReducedMotion = () =>
  typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches

export default function Hero({ items }: { items: Item[] }) {
  const slides = items.filter((i) => i.backdrop).slice(0, 6)
  const [index, setIndex] = useState(0)
  const [paused, setPaused] = useState(prefersReducedMotion)
  const [hold, setHold] = useState(false)

  const current = slides[index]
  if (!current) return null
  const rating = formatRating(current.rating)
  const running = !paused && !hold

  return (
    <section
      className="hero"
      aria-roledescription="Karussell"
      aria-label="Gerade angesagt"
      onMouseEnter={() => setHold(true)}
      onMouseLeave={() => setHold(false)}
      onFocus={() => setHold(true)}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setHold(false)
      }}
    >
      <div className="hero-stage" aria-hidden="true">
        {slides.map((s, i) => (
          <img
            key={`${s.type}-${s.id}`}
            src={s.backdrop ?? undefined}
            alt=""
            className={`hero-img${i === index ? ' is-active' : ''}`}
            fetchPriority={i === 0 ? 'high' : 'low'}
            decoding="async"
          />
        ))}
        <div className="hero-shade" />
        <div className="hero-grain" />
      </div>

      <div className="hero-body" key={`${current.type}-${current.id}`} aria-live={running ? 'off' : 'polite'}>
        <ul className="meta">
          <li className="meta-tag">{typeLabel(current.type)}</li>
          {current.year && <li>{current.year}</li>}
          {rating && (
            <li className="meta-rating">
              <Icon name="star" size={14} />
              {rating}
            </li>
          )}
        </ul>
        <h1 className="hero-title">
          <Link to={titlePath(current)}>{current.title}</Link>
        </h1>
        {current.overview && <p className="hero-overview">{current.overview}</p>}
        <div className="actions">
          <Link to={playStartPath(current)} className="btn btn-primary btn-lg">
            <Icon name="play" size={18} />
            Abspielen
          </Link>
          <Link to={titlePath(current)} className="btn btn-glass btn-lg">
            <Icon name="info" size={18} />
            Details
          </Link>
          <SaveButton item={current} variant="round" />
        </div>
      </div>

      <div className="hero-strip">
        <ol className="hero-picks">
          {slides.map((s, i) => (
            <li key={`${s.type}-${s.id}`}>
              <button
                type="button"
                className={`hero-pick${i === index ? ' is-active' : ''}`}
                onClick={() => setIndex(i)}
                aria-current={i === index}
                aria-label={`${s.title} zeigen`}
              >
                <span className="hero-pick-thumb">
                  <img src={s.backdrop ?? undefined} alt="" loading="lazy" decoding="async" />
                </span>
                <span className="hero-pick-title">{s.title}</span>
                <span className="hero-pick-bar">
                  {i === index && (
                    <span
                      className="hero-pick-fill"
                      style={{ animationPlayState: running ? 'running' : 'paused' }}
                      onAnimationEnd={() => setIndex((x) => (x + 1) % slides.length)}
                    />
                  )}
                </span>
              </button>
            </li>
          ))}
        </ol>
        <button
          type="button"
          className="icon-btn hero-pause"
          onClick={() => setPaused((p) => !p)}
          aria-label={paused ? 'Automatischen Wechsel starten' : 'Automatischen Wechsel anhalten'}
        >
          <Icon name={paused ? 'play' : 'pause'} size={14} />
        </button>
      </div>
    </section>
  )
}

export function HeroSkeleton() {
  return (
    <section className="hero hero-skel" aria-hidden="true">
      <div className="hero-body">
        <div className="skel skel-line short" />
        <div className="skel skel-title" />
        <div className="skel skel-line" />
        <div className="skel skel-line" />
      </div>
    </section>
  )
}
