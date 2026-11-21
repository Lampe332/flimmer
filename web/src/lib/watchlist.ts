import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useLocation } from 'react-router'
import { showUndo } from '../components/Toasts'
import { useI18n } from '../i18n'
import { api, toItem, type Item } from './api'

const KEY = ['watchlist'] as const

export function useWatchlist() {
  return useQuery({ queryKey: KEY, queryFn: api.watchlist, staleTime: 30_000 })
}

type Toggle = { item: Item; add: boolean }

/** Watchlist membership + optimistic add/remove for one title. */
export function useWatchToggle(item: Item | undefined) {
  const qc = useQueryClient()
  const { t } = useI18n()
  const onList = useLocation().pathname === '/merkliste'
  const { data } = useWatchlist()
  const saved = !!item && !!data?.some((x) => x.id === item.id && x.type === item.type)

  const mutation = useMutation({
    mutationFn: ({ item: it, add }: Toggle) =>
      add ? api.watchlistAdd(it) : api.watchlistRemove(it.type, it.id),
    onMutate: async ({ item: it, add }: Toggle) => {
      await qc.cancelQueries({ queryKey: KEY })
      const prev = qc.getQueryData<Item[]>(KEY)
      qc.setQueryData<Item[]>(KEY, (old = []) => {
        const rest = old.filter((x) => !(x.id === it.id && x.type === it.type))
        return add ? [toItem(it), ...rest] : rest
      })
      return { prev }
    },
    onError: (_err, _vars, ctx) => {
      if (ctx?.prev) qc.setQueryData(KEY, ctx.prev)
    },
    onSettled: () => qc.invalidateQueries({ queryKey: KEY }),
  })

  const toggle = () => {
    if (!item) return
    mutation.mutate({ item, add: !saved })
    // On the Merkliste the card is gone at once, so the removal can be taken back for a few seconds.
    if (saved && onList) showUndo(t('wl.removed', { title: item.title }), () => restore(qc, item))
  }

  return { saved, toggle, failed: mutation.isError }
}

/** Puts a removed title back (the card's own hook is gone with the card). */
function restore(qc: ReturnType<typeof useQueryClient>, item: Item) {
  qc.setQueryData<Item[]>(KEY, (old = []) => [toItem(item), ...old.filter((x) => !(x.id === item.id && x.type === item.type))])
  void api.watchlistAdd(item).finally(() => qc.invalidateQueries({ queryKey: KEY }))
}
