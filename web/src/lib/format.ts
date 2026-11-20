const dateFmt = new Intl.DateTimeFormat('de-DE', { day: 'numeric', month: 'short', year: 'numeric' })

export function formatDate(iso: string | null): string | null {
  if (!iso) return null
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? null : dateFmt.format(d)
}

export function formatRuntime(min: number | null): string | null {
  if (!min) return null
  const h = Math.floor(min / 60)
  const m = min % 60
  return h > 0 ? `${h} Std. ${m} Min.` : `${m} Min.`
}

export function formatRating(r: number | null): string | null {
  return r === null ? null : r.toLocaleString('de-DE', { minimumFractionDigits: 1, maximumFractionDigits: 1 })
}

export function typeLabel(type: 'movie' | 'tv'): string {
  return type === 'movie' ? 'Film' : 'Serie'
}

/** Episode has not aired yet (air date in the future). */
export function isUpcoming(iso: string | null): boolean {
  if (!iso) return false
  const d = new Date(iso)
  return !Number.isNaN(d.getTime()) && d.getTime() > Date.now()
}
