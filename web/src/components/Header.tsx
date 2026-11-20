import { useEffect, useRef, useState } from 'react'
import { NavLink, useLocation, useNavigate, useSearchParams } from 'react-router'
import { useI18n, type Key } from '../i18n'
import { useNewEpisodes } from '../lib/progress'
import { isActive, useTorrents } from '../lib/torrents'
import Icon, { type IconName } from './Icon'
import Logo from './Logo'
import SettingsMenu from './SettingsMenu'
import { HeaderVpn } from './Torrents'

const NAV: { to: string; label: Key; end: boolean; icon: IconName }[] = [
  { to: '/', label: 'nav.home', end: true, icon: 'home' },
  { to: '/filme', label: 'nav.movies', end: false, icon: 'film' },
  { to: '/serien', label: 'nav.tv', end: false, icon: 'tv' },
  { to: '/merkliste', label: 'nav.watchlist', end: false, icon: 'bookmark' },
  { to: '/verlauf', label: 'nav.history', end: false, icon: 'clock' },
  { to: '/downloads', label: 'nav.downloads', end: false, icon: 'download' },
]

/** Series with new episodes on the history entry, running downloads on the downloads entry. */
function Badge({ to, fresh, loading }: { to: string; fresh: number; loading: number }) {
  const { t } = useI18n()
  const count = to === '/verlauf' ? fresh : to === '/downloads' ? loading : 0
  if (count === 0) return null
  return (
    <>
      <span className={`nav-badge${to === '/downloads' ? ' is-live' : ''}`} aria-hidden="true">
        {count > 9 ? '9+' : count}
      </span>
      <span className="sr-only">{to === '/verlauf' ? t('nav.newBadge', { n: count }) : t('nav.loadingBadge', { n: count })}</span>
    </>
  )
}

/** True once the page has scrolled past the top, so the bar can turn solid over content. */
function useScrolled(): boolean {
  const [scrolled, setScrolled] = useState(false)
  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 12)
    onScroll()
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => window.removeEventListener('scroll', onScroll)
  }, [])
  return scrolled
}

export default function Header() {
  const navigate = useNavigate()
  const location = useLocation()
  const [params] = useSearchParams()
  const onSearch = location.pathname === '/suche'
  const urlQ = onSearch ? (params.get('q') ?? '') : ''
  const [value, setValue] = useState(urlQ)
  const input = useRef<HTMLInputElement>(null)
  const typed = useRef(false)
  const scrolled = useScrolled()
  const fresh = useNewEpisodes().data?.length ?? 0
  const loading = useTorrents().data?.torrents.filter(isActive).length ?? 0
  const { t } = useI18n()

  // Keep field in sync when the URL changes (back/forward, leaving search).
  useEffect(() => {
    if (!typed.current) setValue(urlQ)
  }, [urlQ])

  // Debounced live search.
  useEffect(() => {
    if (!typed.current) return
    const q = value.trim()
    const t = window.setTimeout(() => {
      typed.current = false
      if (q === urlQ.trim()) return
      if (!q && !onSearch) return
      navigate(q ? `/suche?q=${encodeURIComponent(q)}` : '/suche', { replace: onSearch })
    }, 300)
    return () => window.clearTimeout(t)
  }, [value, urlQ, onSearch, navigate])

  // "/" focuses the search field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== '/' || e.ctrlKey || e.metaKey || e.altKey) return
      const t = e.target as HTMLElement | null
      if (t && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName))) return
      e.preventDefault()
      input.current?.focus()
      input.current?.select()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  return (
    <>
      <header className={`header${scrolled ? ' is-solid' : ''}`}>
        <div className="header-inner">
          <Logo />
          <nav className="nav" aria-label={t('nav.main')}>
            {NAV.map((n) => (
              <NavLink key={n.to} to={n.to} end={n.end} className="nav-link">
                {t(n.label)}
                <Badge to={n.to} fresh={fresh} loading={loading} />
              </NavLink>
            ))}
          </nav>
          <form
            className="search"
            role="search"
            onSubmit={(e) => {
              e.preventDefault()
              typed.current = false
              const q = value.trim()
              if (q) navigate(`/suche?q=${encodeURIComponent(q)}`, { replace: onSearch })
            }}
          >
            <Icon name="search" size={18} />
            <input
              ref={input}
              type="search"
              value={value}
              onChange={(e) => {
                typed.current = true
                setValue(e.target.value)
              }}
              placeholder={t('search.placeholder')}
              aria-label={t('search.placeholder')}
              autoComplete="off"
              spellCheck={false}
              enterKeyHint="search"
            />
            <kbd className="search-kbd" aria-hidden="true">
              /
            </kbd>
          </form>
          <HeaderVpn />
          <SettingsMenu />
        </div>
      </header>

      <nav className="tabbar" aria-label={t('nav.main')}>
        {NAV.map((n) => (
          <NavLink key={n.to} to={n.to} end={n.end} className="tabbar-link">
            <span className="tabbar-icon">
              <Icon name={n.icon} size={22} />
              <Badge to={n.to} fresh={fresh} loading={loading} />
            </span>
            <span>{t(n.label)}</span>
          </NavLink>
        ))}
      </nav>
    </>
  )
}
