import { lazy, Suspense, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { Route, Routes, useLocation, useNavigate, useNavigationType } from 'react-router'
import ErrorBoundary from './components/ErrorBoundary'
import Footer from './components/Footer'
import Header from './components/Header'
import { PageLoader } from './components/States'
import Toasts from './components/Toasts'
import Tour, { startTour } from './components/Tour'
import { useI18n } from './i18n'
import { START_PAGES, getPrefs, loadPrefs } from './lib/prefs'
import { registerShellWorker, useSession } from './lib/session'
import { useSetup } from './lib/setup'
import Home from './pages/Home'
import Pair, { SetupOnPc } from './pages/Pair'

const Browse = lazy(() => import('./pages/Browse'))
const Search = lazy(() => import('./pages/Search'))
const Detail = lazy(() => import('./pages/Detail'))
const Player = lazy(() => import('./pages/Player'))
const Watchlist = lazy(() => import('./pages/Watchlist'))
const History = lazy(() => import('./pages/History'))
const NotFound = lazy(() => import('./pages/NotFound'))
const Downloads = lazy(() => import('./pages/Downloads'))
const TorrentPlay = lazy(() => import('./pages/TorrentPlay'))
const Settings = lazy(() => import('./pages/Settings'))
const Help = lazy(() => import('./pages/Help'))
const Onboarding = lazy(() => import('./pages/Onboarding'))

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

/**
 * First start (no TMDB token, or setup not finished) and `/einrichtung` show the setup flow instead of the app.
 * A TMDB call that answers `needs_setup` (token removed by a reset) brings it back.
 */
function useSetupGate() {
  const setup = useSetup()
  const qc = useQueryClient()
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const [prefsReady, setPrefsReady] = useState(false)
  const startApplied = useRef(false)

  useEffect(() => {
    void loadPrefs().then(() => setPrefsReady(true))
    const onNeeds = () => void qc.invalidateQueries({ queryKey: ['setup'] })
    window.addEventListener('flimmer:needs-setup', onNeeds)
    return () => window.removeEventListener('flimmer:needs-setup', onNeeds)
  }, [qc])

  const needed = !!setup.data && (setup.data.tmdb === 'missing' || !setup.data.onboarded)

  // Start page from the settings, once per app start, only when the app opened on "/".
  useEffect(() => {
    if (startApplied.current || !prefsReady || !setup.data || needed) return
    startApplied.current = true
    const target = START_PAGES.find((p) => p.id === getPrefs().start)?.path ?? '/'
    if (pathname === '/' && target !== '/') navigate(target, { replace: true })
  }, [prefsReady, setup.data, needed, pathname, navigate])

  const finish = async () => {
    await loadPrefs()
    await qc.invalidateQueries()
    navigate('/', { replace: true })
    if (!getPrefs().tourDone) window.setTimeout(startTour, 400)
  }
  return { setup, show: needed || pathname === '/einrichtung', finish }
}

/**
 * In front of everything: who is this page to the server? The desktop window has its session, a paired phone
 * its device token; anybody else gets the pairing page and nothing of the app.
 */
export default function App() {
  // Subscribing here re-renders the whole tree on a language switch.
  useI18n()
  const session = useSession()
  const qc = useQueryClient()
  const role = session.data?.role
  useEffect(() => {
    const onUnpaired = () => void qc.invalidateQueries({ queryKey: ['session'] })
    window.addEventListener('flimmer:unpaired', onUnpaired)
    return () => window.removeEventListener('flimmer:unpaired', onUnpaired)
  }, [qc])
  useEffect(() => {
    if (role === 'device') registerShellWorker()
  }, [role])
  if (session.isPending) return <PageLoader />
  // No answer at all (server gone): the app's own error pages say so.
  if (!session.data) return <Shell device={false} />
  if (session.data.role === 'none') return <Pair session={session.data} />
  return <Shell device={session.data.role === 'device'} />
}

function Shell({ device }: { device: boolean }) {
  const { pathname } = useLocation()
  const { t } = useI18n()
  const gate = useSetupGate()
  const immersive = pathname.startsWith('/play/')
  if (gate.setup.isPending) return <PageLoader />
  // The first-run setup needs the PC: token, VPN file and folders are set there.
  if (gate.show && gate.setup.data && device) return <SetupOnPc />
  if (gate.show && gate.setup.data)
    return (
      <Suspense fallback={<PageLoader />}>
        <Onboarding state={gate.setup.data} onFinish={() => void gate.finish()} />
      </Suspense>
    )
  return (
    <div className={`app${immersive ? ' is-immersive' : ''}`}>
      <a href="#main" className="skip-link">
        {t('app.skip')}
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
            <Route path="/play/torrent/:hash" element={<TorrentPlay />} />
            <Route path="/play/:type/:id" element={<Player />} />
            <Route path="/downloads" element={<Downloads />} />
            <Route path="/merkliste" element={<Watchlist />} />
            <Route path="/verlauf" element={<History />} />
            <Route path="/einstellungen" element={<Settings />} />
            <Route path="/hilfe" element={<Help />} />
            <Route path="*" element={<NotFound />} />
              </Routes>
            </div>
          </Suspense>
        </ErrorBoundary>
      </main>
      <Footer />
      <Tour />
      <Toasts />
    </div>
  )
}
