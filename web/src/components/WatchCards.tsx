import { memo } from 'react'
import { Link } from 'react-router'
import { useI18n } from '../i18n'
import { playPath, type ContinueEntry, type NewEpisode } from '../lib/api'
import { formatDate } from '../lib/format'
import { epCode, ratio, remaining } from '../lib/progress'
import Icon from './Icon'

/** Thin lamp-coloured line along the bottom edge of an image. */
export function ProgressBar({ position, duration }: { position: number; duration: number }) {
  return (
    <span className="progress-bar" aria-hidden="true">
      <span style={{ transform: `scaleX(${ratio(position, duration)})` }} />
    </span>
  )
}

function Art({ src, alt }: { src: string | null; alt: string }) {
  return src ? (
    <img src={src} alt={alt} loading="lazy" decoding="async" width={1280} height={720} />
  ) : (
    <div className="card-noposter" aria-hidden="true">
      <Icon name="film" size={28} />
    </div>
  )
}

/** "Continue watching": landscape card that plays straight away. */
export const ContinueCard = memo(function ContinueCard({ entry, onRemove }: { entry: ContinueEntry; onRemove: () => void }) {
  const { t } = useI18n()
  const { item } = entry
  const code = epCode(entry.season, entry.episode)
  const left = entry.up_next ? null : remaining(entry.position, entry.duration)
  const label = [item.title, code, entry.up_next ? t('watch.nextEpisode') : t('watch.resume')].filter(Boolean).join(', ')
  return (
    <article className="card card-wide card-watch">
      <Link to={playPath(item.type, item.id, entry.season ?? 1, entry.episode ?? 1)} className="card-link" aria-label={label}>
        <div className="card-poster">
          <Art src={item.backdrop ?? entry.still} alt="" />
          <span className="card-watch-play" aria-hidden="true">
            <Icon name="play" size={20} />
          </span>
          <div className="card-wide-text">
            <h3 className="card-title">{item.title}</h3>
            <p className="card-meta">
              {code && <span className="card-ep">{code}</span>}
              {entry.episode_name && <span className="card-ep-name">{entry.episode_name}</span>}
              {entry.up_next ? <span className="card-tag">{t('watch.nextTag')}</span> : left && <span className="card-left">{left}</span>}
            </p>
          </div>
          {!entry.up_next && <ProgressBar position={entry.position} duration={entry.duration} />}
        </div>
      </Link>
      <button
        type="button"
        className="save-chip card-remove"
        aria-label={t('watch.removeLabel', { title: item.title })}
        title={t('watch.removeTitle')}
        onClick={onRemove}
      >
        <Icon name="close" size={15} stroke={2.2} />
      </button>
    </article>
  )
})

/** "New episodes of your shows": the first unseen new episode of a series. */
export const NewEpisodeCard = memo(function NewEpisodeCard({ ep }: { ep: NewEpisode }) {
  const { t } = useI18n()
  const code = epCode(ep.season, ep.episode)
  const date = formatDate(ep.air_date)
  return (
    <article className="card card-wide card-watch">
      <Link
        to={playPath('tv', ep.item.id, ep.season, ep.episode)}
        className="card-link"
        aria-label={t('watch.playEpisode', { title: ep.item.title, code: code ?? '' })}
      >
        <div className="card-poster">
          <Art src={ep.item.backdrop ?? ep.still} alt="" />
          <span className="card-watch-play" aria-hidden="true">
            <Icon name="play" size={20} />
          </span>
          <span className="card-new">{t('watch.newCount', { n: ep.count })}</span>
          <div className="card-wide-text">
            <h3 className="card-title">{ep.item.title}</h3>
            <p className="card-meta">
              <span className="card-ep">{code}</span>
              {ep.name && <span className="card-ep-name">{ep.name}</span>}
              {date && <span className="card-left">{date}</span>}
            </p>
          </div>
        </div>
      </Link>
    </article>
  )
})
