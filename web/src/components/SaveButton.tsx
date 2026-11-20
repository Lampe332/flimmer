import type { Item } from '../lib/api'
import { useWatchToggle } from '../lib/watchlist'
import Icon from './Icon'

type Props = { item: Item; variant?: 'button' | 'chip' | 'round' }

export default function SaveButton({ item, variant = 'button' }: Props) {
  const { saved, toggle } = useWatchToggle(item)

  if (variant === 'button') {
    return (
      <button type="button" className={`btn btn-glass btn-lg${saved ? ' is-saved' : ''}`} aria-pressed={saved} onClick={toggle}>
        <span className="swap" data-on={saved}>
          <Icon name="plus" size={18} />
          <Icon name="check" size={18} />
        </span>
        {saved ? 'Gemerkt' : 'Merken'}
      </button>
    )
  }

  return (
    <button
      type="button"
      className={`${variant === 'chip' ? 'save-chip' : 'btn-round'}${saved ? ' is-saved' : ''}`}
      aria-pressed={saved}
      aria-label={saved ? `${item.title} von der Merkliste entfernen` : `${item.title} merken`}
      title={saved ? 'Von der Merkliste entfernen' : 'Merken'}
      onClick={(e) => {
        e.preventDefault()
        e.stopPropagation()
        toggle()
      }}
    >
      <span className="swap" data-on={saved}>
        <Icon name="plus" size={variant === 'chip' ? 16 : 20} />
        <Icon name="check" size={variant === 'chip' ? 16 : 20} />
      </span>
    </button>
  )
}
