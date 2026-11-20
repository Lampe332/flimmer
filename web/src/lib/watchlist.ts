import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api, toItem, type Item } from './api'

const KEY = ['watchlist'] as const

export function useWatchlist() {
  return useQuery({ queryKey: KEY, queryFn: api.watchlist, staleTime: 30_000 })
}

type Toggle = { item: Item; add: boolean }

/** Watchlist membership + optimistic add/remove for one title. */
export function useWatchToggle(item: Item | undefined) {
  const qc = useQueryClient()
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
    if (item) mutation.mutate({ item, add: !saved })
  }

  return { saved, toggle, failed: mutation.isError }
}
