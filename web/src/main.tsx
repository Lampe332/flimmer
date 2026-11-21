import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserRouter } from 'react-router'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import '@fontsource-variable/bricolage-grotesque/wdth.css'
import '@fontsource-variable/instrument-sans'
import './styles.css'
import './mobile.css'
import { loadInitialLang, subscribeLang } from './i18n'
import { ApiError } from './lib/api'
import App from './App'

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 5 * 60_000,
      gcTime: 30 * 60_000,
      refetchOnWindowFocus: false,
      retry: (count, err) => !(err instanceof ApiError && err.status >= 400 && err.status < 500) && count < 1,
    },
  },
})

/** Data that depends on the UI language (TMDB texts) is fetched again after a switch; the old text stays until the new one arrives. */
const LANG_FREE = new Set(['scan', 'buffer', 'caps', 'settings', 'progress', 'watchlist'])
subscribeLang(() => {
  void queryClient.invalidateQueries({ predicate: (q) => !LANG_FREE.has(String(q.queryKey[0])) })
})

const root = document.getElementById('root')
if (!root) throw new Error('#root is missing')

// The UI texts of the saved language come first; nothing is shown without them.
void loadInitialLang().finally(() =>
  createRoot(root).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <BrowserRouter>
          <App />
        </BrowserRouter>
      </QueryClientProvider>
    </StrictMode>,
  ),
)
