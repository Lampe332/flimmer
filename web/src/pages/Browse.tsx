import { useInfiniteQuery, useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router'
import ResultGrid from '../components/ResultGrid'
import { EmptyState, ErrorState } from '../components/States'
import { useI18n, type Key } from '../i18n'
import { api, type ListKind, type MediaType } from '../lib/api'
import { useTitle } from '../lib/title'

const TABS: Record<MediaType, { id: ListKind; label: Key; hint: Key }[]> = {
  movie: [
    { id: 'popular', label: 'browse.popular', hint: 'browse.popularHint' },
    { id: 'top_rated', label: 'browse.top', hint: 'browse.topHint' },
    { id: 'trending', label: 'browse.trending', hint: 'browse.trendingHint' },
    { id: 'now_playing', label: 'browse.nowPlaying', hint: 'browse.nowPlayingHint' },
  ],
  tv: [
    { id: 'popular', label: 'browse.popular', hint: 'browse.popularHint' },
    { id: 'top_rated', label: 'browse.top', hint: 'browse.topHint' },
    { id: 'trending', label: 'browse.trending', hint: 'browse.trendingHint' },
    { id: 'on_the_air', label: 'browse.onAir', hint: 'browse.onAirHint' },
  ],
}

export default function Browse({ type }: { type: MediaType }) {
  const { t } = useI18n()
  const [params, setParams] = useSearchParams()
  const tabs = TABS[type]
  const listParam = params.get('liste')
  const list: ListKind = tabs.find((x) => x.id === listParam)?.id ?? 'popular'
  const genreRaw = Number(params.get('genre'))
  const genre = Number.isInteger(genreRaw) && genreRaw > 0 ? genreRaw : null

  const genres = useQuery({ queryKey: ['genres', type], queryFn: () => api.genres(type), staleTime: Infinity })

  const q = useInfiniteQuery({
    queryKey: ['list', type, genre === null ? list : 'genre', genre],
    queryFn: ({ pageParam }) => api.list(type, genre === null ? list : 'popular', pageParam, genre),
    initialPageParam: 1,
    getNextPageParam: (last) => (last.page < Math.min(last.total_pages, 500) ? last.page + 1 : undefined),
  })

  const items = q.data?.pages.flatMap((p) => p.results) ?? []
  const heading = t(type === 'movie' ? 'nav.movies' : 'nav.tv')
  useTitle(heading)
  const genreName = genre !== null ? genres.data?.find((g) => g.id === genre)?.name : undefined
  const hint = tabs.find((x) => x.id === list)?.hint

  const setTab = (id: ListKind) => setParams(id === 'popular' ? {} : { liste: id }, { replace: true })
  const setGenre = (id: number | null) => setParams(id === null ? {} : { genre: String(id) }, { replace: true })

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1 className="page-title">{heading}</h1>
          <p className="page-sub">{genreName ? t('browse.genreSorted', { genre: genreName }) : hint && t(hint)}</p>
        </div>
        <div className="tabs" role="tablist" aria-label={t('browse.sort', { heading })}>
          {tabs.map((x) => (
            <button
              key={x.id}
              type="button"
              role="tab"
              aria-selected={genre === null && list === x.id}
              className="tab"
              onClick={() => setTab(x.id)}
            >
              {t(x.label)}
            </button>
          ))}
        </div>
      </div>

      <div className="genres" role="group" aria-label={t('browse.genreFilter')}>
        <button type="button" className="chip" aria-pressed={genre === null} onClick={() => setGenre(null)}>
          {t('browse.allGenres')}
        </button>
        {genres.data?.map((g) => (
          <button
            key={g.id}
            type="button"
            className="chip"
            aria-pressed={genre === g.id}
            onClick={() => setGenre(genre === g.id ? null : g.id)}
          >
            {g.name}
          </button>
        ))}
      </div>

      {q.error ? (
        <ErrorState error={q.error} onRetry={() => void q.refetch()} />
      ) : !q.isPending && items.length === 0 ? (
        <EmptyState title={t('browse.emptyTitle')} text={t('browse.emptyText')} />
      ) : (
        <ResultGrid
          items={items}
          loading={q.isPending}
          hasMore={!!q.hasNextPage}
          loadingMore={q.isFetchingNextPage}
          onMore={() => {
            if (q.hasNextPage && !q.isFetchingNextPage) void q.fetchNextPage()
          }}
        />
      )}
    </div>
  )
}
