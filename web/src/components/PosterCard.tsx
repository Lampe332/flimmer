import { memo } from 'react'
import { Link } from 'react-router'
import { titlePath, type Item } from '../lib/api'
import { formatRating, typeLabel } from '../lib/format'
import Icon from './Icon'
import SaveButton from './SaveButton'

type Props = { item: Item; showType?: boolean }

function PosterCard({ item, showType }: Props) {
  const rating = formatRating(item.rating)
  return (
    <article className="card">
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
              <Icon name="star" size={12} />
              {rating}
            </span>
          )}
        </div>
        <h3 className="card-title">{item.title}</h3>
        <p className="card-meta">
          {item.year ?? 'Jahr unbekannt'}
          {showType && <span className="card-type">{typeLabel(item.type)}</span>}
        </p>
      </Link>
      <SaveButton item={item} variant="chip" />
    </article>
  )
}

export default memo(PosterCard)
