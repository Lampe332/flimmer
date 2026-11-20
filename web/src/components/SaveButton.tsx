import type { Item } from '../lib/api'
import { useWatchToggle } from '../lib/watchlist'
import Icon from './Icon'

type Props = { item: Item; variant?: 'button' | 'chip' }

export default function SaveButton({ item, variant = 'button' }: Props) {
  const { saved, toggle } = useWatchToggle(item)
  const label = saved ? 'Gemerkt' : 'Merken'

  if (variant === 'chip') {
    return (
      <button
        type="button"
        className={`save-chip${saved ? ' is-saved' : ''}`}
        aria-pressed={saved}
        aria-label={saved ? `${item.title} von der Merkliste entfernen` : `${item.title} merken`}
        title={saved ? 'Von der Merkliste entfernen' : 'Merken'}
        onClick={(e) => {
          e.preventDefault()
          e.stopPropagation()
          toggle()
        }}
      >
        <Icon name={saved ? 'check' : 'plus'} size={16} />
      </button>
    )
  }

  return (
    <button
      type="button"
      className={`btn btn-ghost${saved ? ' is-saved' : ''}`}
      aria-pressed={saved}
      onClick={toggle}
    >
      <Icon name={saved ? 'check' : 'plus'} size={18} />
      {label}
    </button>
  )
}
