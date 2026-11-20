import { useEffect, useRef, useState } from 'react'
import { NavLink, useLocation, useNavigate, useSearchParams } from 'react-router'
import { useNewEpisodes } from '../lib/progress'
import Icon, { type IconName } from './Icon'
import Logo from './Logo'

const NAV: { to: string; label: string; end: boolean; icon: IconName }[] = [
  { to: '/', label: 'Start', end: true, icon: 'home' },
  { to: '/filme', label: 'Filme', end: false, icon: 'film' },
  { to: '/serien', label: 'Serien', end: false, icon: 'tv' },
  { to: '/merkliste', label: 'Merkliste', end: false, icon: 'bookmark' },
  { to: '/verlauf', label: 'Verlauf', end: false, icon: 'clock' },
]

/** Series with new episodes, shown on the "Verlauf" entry. */
function Badge({ to, count }: { to: string; count: number }) {
  if (to !== '/verlauf' || count === 0) return null
  return (
    <>
      <span className="nav-badge" aria-hidden="true">
        {count > 9 ? '9+' : count}
      </span>
      <span className="sr-only">{count === 1 ? ', 1 Serie mit neuen Folgen' : `, ${count} Serien mit neuen Folgen`}</span>
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
          <nav className="nav" aria-label="Hauptnavigation">
            {NAV.map((n) => (
              <NavLink key={n.to} to={n.to} end={n.end} className="nav-link">
                {n.label}
                <Badge to={n.to} count={fresh} />
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
              placeholder="Filme und Serien suchen"
              aria-label="Filme und Serien suchen"
              autoComplete="off"
              spellCheck={false}
              enterKeyHint="search"
            />
            <kbd className="search-kbd" aria-hidden="true">
              /
            </kbd>
          </form>
        </div>
      </header>

      <nav className="tabbar" aria-label="Hauptnavigation">
        {NAV.map((n) => (
          <NavLink key={n.to} to={n.to} end={n.end} className="tabbar-link">
            <span className="tabbar-icon">
              <Icon name={n.icon} size={22} />
              <Badge to={n.to} count={fresh} />
            </span>
            <span>{n.label}</span>
          </NavLink>
        ))}
      </nav>
    </>
  )
}
