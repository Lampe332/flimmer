import { lazy, Suspense, useEffect } from 'react'
import { Route, Routes, useLocation } from 'react-router'
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
const NotFound = lazy(() => import('./pages/NotFound'))

function ScrollToTop() {
  const { pathname } = useLocation()
  useEffect(() => {
    window.scrollTo(0, 0)
  }, [pathname])
  return null
}

export default function App() {
  const { pathname } = useLocation()
  return (
    <div className="app">
      <a href="#main" className="skip-link">
        Zum Inhalt springen
      </a>
      <ScrollToTop />
      <Header />
      <main id="main" className="main">
        <ErrorBoundary key={pathname}>
        <Suspense fallback={<PageLoader />}>
          <Routes>
            <Route path="/" element={<Home />} />
            <Route path="/filme" element={<Browse type="movie" />} />
            <Route path="/serien" element={<Browse type="tv" />} />
            <Route path="/suche" element={<Search />} />
            <Route path="/titel/:type/:id" element={<Detail />} />
            <Route path="/play/:type/:id" element={<Player />} />
            <Route path="/merkliste" element={<Watchlist />} />
            <Route path="*" element={<NotFound />} />
          </Routes>
        </Suspense>
        </ErrorBoundary>
      </main>
      <Footer />
    </div>
  )
}
