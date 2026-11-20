import { keepPreviousData, useInfiniteQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router'
import ResultGrid from '../components/ResultGrid'
import { EmptyState, ErrorState } from '../components/States'
import { api } from '../lib/api'
import { useTitle } from '../lib/title'

export default function Search() {
  const [params] = useSearchParams()
  const q = (params.get('q') ?? '').trim()
  useTitle(q ? `Suche: ${q}` : 'Suche')

  const res = useInfiniteQuery({
    queryKey: ['search', q],
    queryFn: ({ pageParam }) => api.search(q, pageParam),
    initialPageParam: 1,
    getNextPageParam: (last) => (last.page < last.total_pages ? last.page + 1 : undefined),
    enabled: q.length > 0,
    placeholderData: keepPreviousData,
  })

  const items = res.data?.pages.flatMap((p) => p.results) ?? []

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1 className="page-title">{q ? `„${q}“` : 'Suche'}</h1>
          {q && <p className="page-sub">Filme und Serien zu deiner Suche</p>}
        </div>
      </div>
      {!q ? (
        <EmptyState icon="search" title="Wonach suchst du?" text="Tipp oben einen Film- oder Serientitel ein. Mit / springst du jederzeit ins Suchfeld." />
      ) : res.error ? (
        <ErrorState error={res.error} onRetry={() => void res.refetch()} />
      ) : !res.isPending && !res.isPlaceholderData && items.length === 0 ? (
        <EmptyState icon="search" title="Nichts gefunden" text={`Zu „${q}“ gibt es keine Filme oder Serien. Prüf die Schreibweise oder versuch den Originaltitel.`} />
      ) : (
        <ResultGrid
          items={items}
          loading={res.isPending}
          hasMore={!!res.hasNextPage}
          loadingMore={res.isFetchingNextPage}
          onMore={() => {
            if (res.hasNextPage && !res.isFetchingNextPage) void res.fetchNextPage()
          }}
          showType
        />
      )}
    </div>
  )
}
