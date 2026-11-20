import { useMutation, useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query'
import { t } from '../i18n'
import { api, type HistoryEntry, type MediaType, type Progress, type Season, type WatchMeta } from './api'

export type Ep = { s: number; e: number }

export const progressKey = (type: MediaType, id: number) => ['progress', type, id] as const

/** Everything that changes when a position is saved or an episode is marked. */
export function invalidateWatching(qc: QueryClient, type: MediaType, id: number) {
  void qc.invalidateQueries({ queryKey: progressKey(type, id) })
  void qc.invalidateQueries({ queryKey: ['continue'] })
  void qc.invalidateQueries({ queryKey: ['history'] })
  void qc.invalidateQueries({ queryKey: ['new-episodes'] })
}

export function useProgress(type: MediaType, id: number, enabled = true) {
  return useQuery({ queryKey: progressKey(type, id), queryFn: () => api.progress(type, id), enabled, staleTime: 5_000 })
}

export function useContinue() {
  return useQuery({ queryKey: ['continue'], queryFn: api.continueList, staleTime: 10_000 })
}

export function useHistory() {
  return useQuery({ queryKey: ['history'], queryFn: api.history, staleTime: 10_000 })
}

export function useNewEpisodes() {
  return useQuery({ queryKey: ['new-episodes'], queryFn: api.newEpisodes, staleTime: 10 * 60_000, retry: false })
}

/** Episode before/after `cur` in the season list (crosses season boundaries). */
export function neighbour(seasons: Season[], cur: Ep, dir: 1 | -1): Ep | null {
  const idx = seasons.findIndex((x) => x.season_number === cur.s)
  const here = seasons[idx]
  if (!here) return null
  if (dir === 1) {
    if (cur.e < here.episode_count) return { s: cur.s, e: cur.e + 1 }
    const next = seasons[idx + 1]
    return next && next.episode_count > 0 ? { s: next.season_number, e: 1 } : null
  }
  if (cur.e > 1) return { s: cur.s, e: cur.e - 1 }
  const prev = seasons[idx - 1]
  return prev && prev.episode_count > 0 ? { s: prev.season_number, e: prev.episode_count } : null
}

/** Where "Weiterschauen" on a title page continues: the last episode if unfinished, else the one after it. */
export function resumeTarget(rows: Progress[], seasons: Season[]): (Ep & { row?: Progress }) | null {
  const last = [...rows].sort((a, z) => z.updated_at.localeCompare(a.updated_at))[0]
  if (!last) return null
  if (last.season === null || last.episode === null) return null
  const cur = { s: last.season, e: last.episode }
  if (!last.finished) return { ...cur, row: last }
  let next = neighbour(seasons, cur, 1)
  for (let i = 0; next && i < 50; i += 1) {
    const n = next
    const row = rows.find((r) => r.season === n.s && r.episode === n.e)
    if (!row?.finished) return { ...n, row }
    next = neighbour(seasons, n, 1)
  }
  return null
}

export function epCode(s: number | null, e: number | null): string | null {
  return s === null || e === null ? null : `S${s}E${e}`
}

/** "noch 34 Min." / "34 min left" */
export function remaining(position: number, duration: number): string | null {
  if (duration <= 0) return null
  const min = Math.max(1, Math.round((duration - position) / 60))
  return t('fmt.left', { m: min })
}

export function ratio(position: number, duration: number): number {
  return duration > 0 ? Math.min(1, Math.max(0, position / duration)) : 0
}

type Mark = { s: number; e: number; watched: boolean; meta: WatchMeta }

/** Mark one episode watched / not watched, optimistic. */
export function useMarkEpisode(id: number) {
  const qc = useQueryClient()
  const key = progressKey('tv', id)
  return useMutation({
    mutationFn: ({ s, e, watched, meta }: Mark) =>
      watched ? api.markWatched('tv', id, s, e, meta) : api.markUnwatched('tv', id, s, e),
    onMutate: async ({ s, e, watched }: Mark) => {
      await qc.cancelQueries({ queryKey: key })
      const prev = qc.getQueryData<Progress[]>(key)
      qc.setQueryData<Progress[]>(key, (old = []) => {
        const rest = old.filter((r) => !(r.season === s && r.episode === e))
        if (!watched) return rest
        const was = old.find((r) => r.season === s && r.episode === e)
        return [
          ...rest,
          {
            type: 'tv',
            id,
            season: s,
            episode: e,
            position: was?.duration ?? 0,
            duration: was?.duration ?? 0,
            finished: true,
            updated_at: new Date().toISOString(),
          },
        ]
      })
      return { prev }
    },
    onError: (_err, _vars, ctx) => {
      if (ctx?.prev) qc.setQueryData(key, ctx.prev)
    },
    onSettled: () => invalidateWatching(qc, 'tv', id),
  })
}

export function useHideContinue() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ type, id }: { type: MediaType; id: number }) => api.continueHide(type, id),
    onMutate: async ({ type, id }) => {
      await qc.cancelQueries({ queryKey: ['continue'] })
      qc.setQueryData<{ item: { type: MediaType; id: number } }[]>(['continue'], (old = []) =>
        old.filter((c) => !(c.item.type === type && c.item.id === id)),
      )
    },
    onSettled: () => qc.invalidateQueries({ queryKey: ['continue'] }),
  })
}

export function useHistoryDelete() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (entry: number | 'all') => (entry === 'all' ? api.historyClear() : api.historyDelete(entry)),
    onMutate: async (entry) => {
      await qc.cancelQueries({ queryKey: ['history'] })
      const prev = qc.getQueryData<HistoryEntry[]>(['history'])
      qc.setQueryData<HistoryEntry[]>(['history'], (old = []) => (entry === 'all' ? [] : old.filter((h) => h.entry !== entry)))
      return { prev }
    },
    onError: (_err, _vars, ctx) => {
      if (ctx?.prev) qc.setQueryData(['history'], ctx.prev)
    },
    onSettled: () => qc.invalidateQueries({ queryKey: ['history'] }),
  })
}
