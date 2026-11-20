import { memo } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router'
import { api, playStartPath, titlePath, type Item } from '../lib/api'
import { formatRating, typeLabel } from '../lib/format'
import Icon from './Icon'
import SaveButton from './SaveButton'

type Props = { item: Item; showType?: boolean }

/** Load the title page's data while the pointer rests on a card, so the click opens it instantly. */
function useWarm(item: Item) {
  const qc = useQueryClient()
  return () =>
    void qc.prefetchQuery({ queryKey: ['title', item.type, item.id], queryFn: () => api.title(item.type, item.id) })
}

/** Round play button in the middle of the artwork: straight to the player, skipping the title page. */
function PlayChip({ item }: { item: Item }) {
  return (
    <span className="card-play-area">
      <Link
        to={playStartPath(item)}
        className="card-play"
        aria-label={`${item.title} abspielen`}
        title={item.type === 'tv' ? 'Abspielen (weiter, wo du warst)' : 'Abspielen'}
      >
        <Icon name="play" size={20} />
      </Link>
    </span>
  )
}

function PosterCard({ item, showType }: Props) {
  const rating = formatRating(item.rating)
  const warm = useWarm(item)
  return (
    <article className="card" onPointerEnter={warm} onFocus={warm}>
      <Link to={titlePath(item)} className="card-link">
        <div className="card-poster">
          {item.poster ? (
            <img src={item.poster} alt="" loading="lazy" decoding="async" width={500} height={750} />
          ) : (
            <div className="card-noposter" aria-hidden="true">
              <Icon name="film" size={28} />
            </div>
          )}
          {rating && (
            <span className="card-rating" aria-label={`Bewertung ${rating}`}>
              <Icon name="star" size={11} />
              {rating}
            </span>
          )}
        </div>
        <h3 className="card-title">{item.title}</h3>
        <p className="card-meta">
          <span>{item.year ?? 'Jahr unbekannt'}</span>
          {showType && <span className="card-type">{typeLabel(item.type)}</span>}
        </p>
      </Link>
      <PlayChip item={item} />
      <SaveButton item={item} variant="chip" />
    </article>
  )
}

export default memo(PosterCard)

/** Landscape card on the backdrop, used for the trending shelf. */
export const WideCard = memo(function WideCard({ item }: { item: Item }) {
  const rating = formatRating(item.rating)
  const warm = useWarm(item)
  return (
    <article className="card card-wide" onPointerEnter={warm} onFocus={warm}>
      <Link to={titlePath(item)} className="card-link">
        <div className="card-poster">
          {item.backdrop ? (
            <img src={item.backdrop} alt="" loading="lazy" decoding="async" width={1280} height={720} />
          ) : (
            <div className="card-noposter" aria-hidden="true">
              <Icon name="film" size={28} />
            </div>
          )}
          <div className="card-wide-text">
            <h3 className="card-title">{item.title}</h3>
            <p className="card-meta">
              <span className="card-type">{typeLabel(item.type)}</span>
              {item.year && <span>{item.year}</span>}
              {rating && (
                <span className="card-wide-rating">
                  <Icon name="star" size={11} />
                  {rating}
                </span>
              )}
            </p>
          </div>
        </div>
      </Link>
      <PlayChip item={item} />
      <SaveButton item={item} variant="chip" />
    </article>
  )
})

export function WideSkeletons({ count = 5 }: { count?: number }) {
  return (
    <>
      {Array.from({ length: count }, (_, i) => (
        <div key={i} className="card card-wide card-skel" aria-hidden="true">
          <div className="card-poster skel" />
        </div>
      ))}
    </>
  )
}
