import { useEffect, useRef, useState } from 'react'
import { NavLink, useLocation, useNavigate, useSearchParams } from 'react-router'
import Icon from './Icon'
import Logo from './Logo'

const NAV = [
  { to: '/', label: 'Start', end: true },
  { to: '/filme', label: 'Filme', end: false },
  { to: '/serien', label: 'Serien', end: false },
  { to: '/merkliste', label: 'Merkliste', end: false },
]

export default function Header() {
  const navigate = useNavigate()
  const location = useLocation()
  const [params] = useSearchParams()
  const onSearch = location.pathname === '/suche'
  const urlQ = onSearch ? (params.get('q') ?? '') : ''
  const [value, setValue] = useState(urlQ)
  const input = useRef<HTMLInputElement>(null)
  const typed = useRef(false)

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
    <header className="header">
      <div className="header-inner">
        <Logo />
        <nav className="nav" aria-label="Hauptnavigation">
          {NAV.map((n) => (
            <NavLink key={n.to} to={n.to} end={n.end} className="nav-link">
              {n.label}
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
          />
          <kbd className="search-kbd" aria-hidden="true">
            /
          </kbd>
        </form>
      </div>
    </header>
  )
}
