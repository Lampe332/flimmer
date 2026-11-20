import { useInfiniteQuery, useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router'
import ResultGrid from '../components/ResultGrid'
import { EmptyState, ErrorState } from '../components/States'
import { api, type ListKind, type MediaType } from '../lib/api'

const TABS: Record<MediaType, { id: ListKind; label: string }[]> = {
  movie: [
    { id: 'popular', label: 'Beliebt' },
    { id: 'top_rated', label: 'Top bewertet' },
    { id: 'trending', label: 'Trending' },
    { id: 'now_playing', label: 'Im Kino' },
  ],
  tv: [
    { id: 'popular', label: 'Beliebt' },
    { id: 'top_rated', label: 'Top bewertet' },
    { id: 'trending', label: 'Trending' },
    { id: 'on_the_air', label: 'Läuft gerade' },
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
  const genreName = genre !== null ? genres.data?.find((g) => g.id === genre)?.name : undefined

  const setTab = (id: ListKind) => setParams(id === 'popular' ? {} : { liste: id }, { replace: true })
  const setGenre = (id: number | null) => setParams(id === null ? {} : { genre: String(id) }, { replace: true })

  return (
    <div className="page">
      <div className="page-head">
        <h1 className="page-title">{heading}</h1>
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

      <div className="genres" aria-label="Nach Genre filtern">
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
      {genreName && <p className="page-note">{genreName}, nach Beliebtheit sortiert</p>}

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
