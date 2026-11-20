import { useEffect } from 'react'

/** Sets the tab title to "<page> | Flimmer" while the page is shown. */
export function useTitle(page: string | undefined): void {
  useEffect(() => {
    document.title = page ? `${page} | Flimmer` : 'Flimmer'
    return () => {
      document.title = 'Flimmer'
    }
  }, [page])
}
