import { createElement, lazy, Suspense, type ComponentType, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { Route, Routes, useLocation, useNavigate, useNavigationType } from 'react-router'
import ErrorBoundary from './components/ErrorBoundary'
import Footer from './components/Footer'
import { usePhonePlayer } from './lib/platform'
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

/**
 * A lazy page that renders straight away once `preload` has fetched it. A plain `lazy` still suspends on its first
 * render even when the chunk is cached, and React holds that fallback for about 300 ms.
 */
function page<P extends object>(load: () => Promise<{ default: ComponentType<P> }>) {
  let loaded: ComponentType<P> | undefined
  let pending: Promise<{ default: ComponentType<P> }> | undefined
  const preload = () =>
    (pending ??= load().then((m) => {
      loaded = m.default
      return m
    }))
  const Lazy = lazy(preload)
  const Page = (props: P) => (loaded ? createElement(loaded, props) : createElement(Lazy, props))
  return Object.assign(Page, { preload })
}

const Browse = page(() => import('./pages/Browse'))
const Search = page(() => import('./pages/Search'))
const Detail = page(() => import('./pages/Detail'))
const Player = lazy(() => import('./pages/Player'))
const Watchlist = page(() => import('./pages/Watchlist'))
const History = page(() => import('./pages/History'))
const NotFound = lazy(() => import('./pages/NotFound'))
const Downloads = page(() => import('./pages/Downloads'))
const TorrentPlay = lazy(() => import('./pages/TorrentPlay'))
const Settings = page(() => import('./pages/Settings'))
const Help = lazy(() => import('./pages/Help'))
const Onboarding = lazy(() => import('./pages/Onboarding'))

/** The pages behind the tab bar, fetched while the app is idle so the first tap does not wait for a chunk. */
function usePrefetchPages() {
  useEffect(() => {
    const load = () => {
      for (const p of [Browse, Watchlist, History, Downloads, Search, Detail, Settings]) void p.preload()
    }
    // The timeout makes sure it runs even when the browser never reports idle time (background tab, busy phone).
    const idle = window.requestIdleCallback
      ? (fn: () => void) => window.requestIdleCallback(fn, { timeout: 2000 })
      : (fn: () => void) => window.setTimeout(fn, 1500)
    const id = idle(load)
    return () => (window.cancelIdleCallback ?? window.clearTimeout)(id)
  }, [])
}

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
  // Phone player: no header, footer or tab bar, the video owns the screen (like Seanime Mobile).
  const bare = usePhonePlayer() && immersive
  usePrefetchPages()
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
    <div className={`app${immersive ? ' is-immersive' : ''}${bare ? ' is-bare' : ''}${pathname === '/suche' ? ' is-search' : ''}`}>
      <a href="#main" className="skip-link">
        {t('app.skip')}
      </a>
      <ScrollManager />
      {!bare && <Header />}
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
      {!bare && <Footer />}
      <Tour />
      <Toasts />
    </div>
  )
}
