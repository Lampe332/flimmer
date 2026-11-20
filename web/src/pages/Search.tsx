import { keepPreviousData, useInfiniteQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router'
import ResultGrid from '../components/ResultGrid'
import { EmptyState, ErrorState } from '../components/States'
import { useI18n } from '../i18n'
import { api } from '../lib/api'
import { useTitle } from '../lib/title'

export default function Search() {
  const { t } = useI18n()
  const [params] = useSearchParams()
  const q = (params.get('q') ?? '').trim()
  useTitle(q ? t('search.tab', { q }) : t('nav.search'))

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
          <h1 className="page-title">{q ? t('search.quoted', { q }) : t('nav.search')}</h1>
          {q && <p className="page-sub">{t('search.sub')}</p>}
        </div>
      </div>
      {!q ? (
        <EmptyState icon="search" title={t('search.emptyTitle')} text={t('search.emptyText')} />
      ) : res.error ? (
        <ErrorState error={res.error} onRetry={() => void res.refetch()} />
      ) : !res.isPending && !res.isPlaceholderData && items.length === 0 ? (
        <EmptyState icon="search" title={t('search.noneTitle')} text={t('search.noneText', { q })} />
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
