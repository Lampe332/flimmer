import { useInfiniteQuery, useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router'
import ResultGrid from '../components/ResultGrid'
import { EmptyState, ErrorState } from '../components/States'
import { api, type ListKind, type MediaType } from '../lib/api'
import { useTitle } from '../lib/title'

const TABS: Record<MediaType, { id: ListKind; label: string; hint: string }[]> = {
  movie: [
    { id: 'popular', label: 'Beliebt', hint: 'Was gerade am meisten geschaut wird' },
    { id: 'top_rated', label: 'Top bewertet', hint: 'Die bestbewerteten Titel bei TMDB' },
    { id: 'trending', label: 'Trending', hint: 'Heute besonders gefragt' },
    { id: 'now_playing', label: 'Im Kino', hint: 'Läuft aktuell auf der großen Leinwand' },
  ],
  tv: [
    { id: 'popular', label: 'Beliebt', hint: 'Was gerade am meisten geschaut wird' },
    { id: 'top_rated', label: 'Top bewertet', hint: 'Die bestbewerteten Titel bei TMDB' },
    { id: 'trending', label: 'Trending', hint: 'Heute besonders gefragt' },
    { id: 'on_the_air', label: 'Läuft gerade', hint: 'Serien mit neuen Folgen in diesen Wochen' },
  ],
}

export default function Browse({ type }: { type: MediaType }) {
  const [params, setParams] = useSearchParams()
  const tabs = TABS[type]
  const listParam = params.get('liste')
  const list: ListKind = tabs.find((t) => t.id === listParam)?.id ?? 'popular'
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
  const heading = type === 'movie' ? 'Filme' : 'Serien'
  useTitle(heading)
  const genreName = genre !== null ? genres.data?.find((g) => g.id === genre)?.name : undefined

  const setTab = (id: ListKind) => setParams(id === 'popular' ? {} : { liste: id }, { replace: true })
  const setGenre = (id: number | null) => setParams(id === null ? {} : { genre: String(id) }, { replace: true })

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1 className="page-title">{heading}</h1>
          <p className="page-sub">{genreName ? `${genreName}, nach Beliebtheit sortiert` : tabs.find((t) => t.id === list)?.hint}</p>
        </div>
        <div className="tabs" role="tablist" aria-label={`${heading} sortieren`}>
          {tabs.map((t) => (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={genre === null && list === t.id}
              className="tab"
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>
      </div>

      <div className="genres" role="group" aria-label="Nach Genre filtern">
        <button type="button" className="chip" aria-pressed={genre === null} onClick={() => setGenre(null)}>
          Alle Genres
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
        <EmptyState title="Keine Titel" text="In dieser Auswahl gibt es gerade nichts. Probier ein anderes Genre." />
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
