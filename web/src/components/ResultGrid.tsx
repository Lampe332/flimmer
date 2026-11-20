import { useEffect, useRef } from 'react'
import type { Item } from '../lib/api'
import PosterCard from './PosterCard'
import { CardSkeletons } from './States'

type Props = {
  items: Item[]
  loading: boolean
  hasMore: boolean
  loadingMore: boolean
  onMore: () => void
  showType?: boolean
}

/** Poster grid with auto-loading sentinel plus an explicit "Mehr laden" button. */
export default function ResultGrid({ items, loading, hasMore, loadingMore, onMore, showType }: Props) {
  const sentinel = useRef<HTMLDivElement>(null)
  const more = useRef(onMore)
  more.current = onMore

  useEffect(() => {
    const el = sentinel.current
    if (!el || !hasMore) return
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) more.current()
      },
      { rootMargin: '600px 0px' },
    )
    io.observe(el)
    return () => io.disconnect()
  }, [hasMore, items.length])

  // Dedupe: TMDB pages can overlap when popularity shifts between requests.
  const seen = new Set<string>()
  const unique = items.filter((i) => {
    const k = `${i.type}-${i.id}`
    if (seen.has(k)) return false
    seen.add(k)
    return true
  })

  return (
    <>
      <div className="grid">
        {loading ? (
          <CardSkeletons count={18} />
        ) : (
          unique.map((item) => <PosterCard key={`${item.type}-${item.id}`} item={item} showType={showType} />)
        )}
        {loadingMore && <CardSkeletons count={6} />}
      </div>
      {hasMore && !loading && (
        <div className="grid-more" ref={sentinel}>
          <button type="button" className="btn btn-glass" onClick={onMore} disabled={loadingMore}>
            {loadingMore ? 'Lädt …' : 'Mehr laden'}
          </button>
        </div>
      )}
    </>
  )
}
