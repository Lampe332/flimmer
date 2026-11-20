import { lazy, Suspense, useEffect, useLayoutEffect, useRef } from 'react'
import { Route, Routes, useLocation, useNavigationType } from 'react-router'
import ErrorBoundary from './components/ErrorBoundary'
import Footer from './components/Footer'
import Header from './components/Header'
import { PageLoader } from './components/States'
import Home from './pages/Home'

const Browse = lazy(() => import('./pages/Browse'))
const Search = lazy(() => import('./pages/Search'))
const Detail = lazy(() => import('./pages/Detail'))
const Player = lazy(() => import('./pages/Player'))
const Watchlist = lazy(() => import('./pages/Watchlist'))
const History = lazy(() => import('./pages/History'))
const NotFound = lazy(() => import('./pages/NotFound'))

/** Scroll position per history entry, so "back" lands where you left the page. */
const positions = new Map<string, number>()

function ScrollManager() {
  const { key } = useLocation()
  const nav = useNavigationType()
  const current = useRef(key)

  useEffect(() => {
    window.history.scrollRestoration = 'manual'
    const onScroll = () => positions.set(current.current, window.scrollY)
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => window.removeEventListener('scroll', onScroll)
  }, [])

  useLayoutEffect(() => {
    current.current = key
    // REPLACE (season tab, typing in the search) keeps the position; a new page starts at the top.
    if (nav === 'REPLACE') return
    if (nav === 'PUSH') {
      window.scrollTo(0, 0)
      return
    }
    // Back/forward: the page may still be loading, so keep trying until it is tall enough (about 1 s).
    const y = positions.get(key) ?? 0
    let tries = 0
    let frame = 0
    const go = () => {
      window.scrollTo(0, y)
      if (Math.abs(window.scrollY - y) > 2 && tries++ < 60) frame = requestAnimationFrame(go)
    }
    go()
    return () => cancelAnimationFrame(frame)
  }, [key, nav])
  return null
}

export default function App() {
  const { pathname } = useLocation()
  const immersive = pathname.startsWith('/play/')
  return (
    <div className={`app${immersive ? ' is-immersive' : ''}`}>
      <a href="#main" className="skip-link">
        Zum Inhalt springen
      </a>
      <ScrollManager />
      <Header />
      <main id="main" className="main">
        <ErrorBoundary key={pathname}>
          <Suspense fallback={<PageLoader />}>
            <div className="route" key={pathname}>
              <Routes>
            <Route path="/" element={<Home />} />
            <Route path="/filme" element={<Browse type="movie" />} />
            <Route path="/serien" element={<Browse type="tv" />} />
            <Route path="/suche" element={<Search />} />
            <Route path="/titel/:type/:id" element={<Detail />} />
            <Route path="/play/:type/:id" element={<Player />} />
            <Route path="/merkliste" element={<Watchlist />} />
            <Route path="/verlauf" element={<History />} />
            <Route path="*" element={<NotFound />} />
              </Routes>
            </div>
          </Suspense>
        </ErrorBoundary>
      </main>
      <Footer />
    </div>
  )
}
