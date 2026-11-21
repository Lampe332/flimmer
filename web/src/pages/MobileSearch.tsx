import { keepPreviousData, useInfiniteQuery } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router'
import Icon from '../components/Icon'
import ResultGrid from '../components/ResultGrid'
import { EmptyState, ErrorState } from '../components/States'
import { useI18n } from '../i18n'
import { api } from '../lib/api'
import { useTitle } from '../lib/title'

const RECENT_KEY = 'flimmer.recentSearches'
const RECENT_MAX = 8

function readRecent(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]')
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string').slice(0, RECENT_MAX) : []
  } catch {
    return []
  }
}
function writeRecent(list: string[]) {
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(list))
  } catch {
    // private mode: the list just is not kept
  }
}

/**
 * Search on a phone: a full-screen page with its own large field (the header hides), the last searches while the
 * field is empty, and live results as you type.
 */
export default function MobileSearch() {
  const { t } = useI18n()
  const navigate = useNavigate()
  const [params, setParams] = useSearchParams()
  const urlQ = params.get('q') ?? ''
  const [value, setValue] = useState(urlQ)
  const [recent, setRecent] = useState(readRecent)
  const input = useRef<HTMLInputElement>(null)
  const q = urlQ.trim()
  useTitle(q ? t('search.tab', { q }) : t('nav.search'))

  useEffect(() => {
    if (!urlQ) input.current?.focus()
  }, []) // only when the page opens

  // live: the URL follows the field 250 ms after the last key (replace, so Back leaves the search)
  useEffect(() => {
    const next = value.trim()
    if (next === q) return
    const timer = window.setTimeout(() => setParams(next ? { q: next } : {}, { replace: true }), 250)
    return () => window.clearTimeout(timer)
  }, [value, q, setParams])

  const remember = (term: string) => {
    const s = term.trim()
    if (!s) return
    const list = [s, ...recent.filter((x) => x.toLowerCase() !== s.toLowerCase())].slice(0, RECENT_MAX)
    setRecent(list)
    writeRecent(list)
  }
  const forget = (term: string) => {
    const list = recent.filter((x) => x !== term)
    setRecent(list)
    writeRecent(list)
  }
  const pick = (term: string) => {
    setValue(term)
    setParams({ q: term }, { replace: true })
    remember(term)
    input.current?.blur()
  }

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
    <div className="msearch">
      <form
        className="msearch-bar"
        role="search"
        onSubmit={(e) => {
          e.preventDefault()
          pick(value)
        }}
      >
        <button
          type="button"
          className="msearch-icon"
          onClick={() => (window.history.length > 1 ? navigate(-1) : navigate('/'))}
          aria-label={t('m.back')}
        >
          <Icon name="arrowLeft" size={22} />
        </button>
        <input
          ref={input}
          type="search"
          className="msearch-input"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder={t('search.placeholder')}
          aria-label={t('search.placeholder')}
          autoComplete="off"
          autoCorrect="off"
          spellCheck={false}
          enterKeyHint="search"
        />
        {value && (
          <button
            type="button"
            className="msearch-icon"
            onClick={() => {
              setValue('')
              setParams({}, { replace: true })
              input.current?.focus()
            }}
            aria-label={t('m.clear')}
          >
            <Icon name="close" size={20} />
          </button>
        )}
      </form>

      {!q ? (
        recent.length > 0 ? (
          <section className="msearch-recent" aria-labelledby="msearch-recent-h">
            <div className="msearch-recent-head">
              <h2 id="msearch-recent-h">{t('m.recent')}</h2>
              <button type="button" className="link-btn" onClick={() => (setRecent([]), writeRecent([]))}>
                {t('m.clearAll')}
              </button>
            </div>
            <ul>
              {recent.map((term) => (
                <li key={term} className="msearch-recent-row">
                  <button type="button" className="msearch-recent-term" onClick={() => pick(term)}>
                    <Icon name="clock" size={18} />
                    <span>{term}</span>
                  </button>
                  <button type="button" className="msearch-icon" onClick={() => forget(term)} aria-label={t('m.forget', { q: term })}>
                    <Icon name="close" size={16} />
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ) : (
          <EmptyState icon="search" title={t('search.emptyTitle')} text={t('m.searchHint')} />
        )
      ) : res.error ? (
        <ErrorState error={res.error} onRetry={() => void res.refetch()} />
      ) : !res.isPending && !res.isPlaceholderData && items.length === 0 ? (
        <EmptyState icon="search" title={t('search.noneTitle')} text={t('search.noneText', { q })} />
      ) : (
        // A tap on a result keeps the term in the recent list.
        <div className="msearch-results" onClickCapture={(e) => (e.target as HTMLElement).closest('a') && remember(q)}>
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
        </div>
      )}
    </div>
  )
}
