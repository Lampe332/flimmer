import { useId, type KeyboardEvent, type ReactNode } from 'react'

/** On/off switch with its label and an optional hint. */
export function Toggle(p: { label: string; hint?: string; checked: boolean; onChange: (on: boolean) => void; disabled?: boolean }) {
  const id = useId()
  return (
    <div className="pref-row">
      <div className="pref-text">
        <span className="pref-label" id={`${id}-l`}>
          {p.label}
        </span>
        {p.hint && (
          <span className="field-hint" id={`${id}-h`}>
            {p.hint}
          </span>
        )}
      </div>
      <button
        type="button"
        role="switch"
        className="switch"
        aria-checked={p.checked}
        aria-labelledby={`${id}-l`}
        aria-describedby={p.hint ? `${id}-h` : undefined}
        disabled={p.disabled}
        onClick={() => p.onChange(!p.checked)}
      >
        <span className="switch-knob" aria-hidden="true" />
      </button>
    </div>
  )
}

/** A row with a label (and hint) on the left and any control on the right. */
export function PrefRow(p: { label: string; hint?: string; children: ReactNode; htmlFor?: string }) {
  return (
    <div className="pref-row">
      <div className="pref-text">
        {p.htmlFor ? (
          <label className="pref-label" htmlFor={p.htmlFor}>
            {p.label}
          </label>
        ) : (
          <span className="pref-label">{p.label}</span>
        )}
        {p.hint && <span className="field-hint">{p.hint}</span>}
      </div>
      <div className="pref-control">{p.children}</div>
    </div>
  )
}

/** Radio group drawn as chips; arrows move the choice like native radios. */
export function Segmented<T extends string | number>(p: {
  label: string
  value: T
  options: { id: T; label: string; swatch?: string }[]
  onChange: (v: T) => void
}) {
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const keys = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End']
    if (!keys.includes(e.key)) return
    e.preventDefault()
    const at = p.options.findIndex((o) => o.id === p.value)
    const last = p.options.length - 1
    const back = e.key === 'ArrowLeft' || e.key === 'ArrowUp'
    const i = e.key === 'Home' ? 0 : e.key === 'End' ? last : (at + (back ? -1 : 1) + p.options.length) % p.options.length
    const next = p.options[i]
    if (!next) return
    p.onChange(next.id)
    const group = e.currentTarget
    requestAnimationFrame(() => group.querySelector<HTMLElement>('[aria-checked="true"]')?.focus())
  }
  return (
    <div className="seg" role="radiogroup" aria-label={p.label} onKeyDown={onKey}>
      {p.options.map((o) => (
        <button
          key={String(o.id)}
          type="button"
          role="radio"
          className="seg-item"
          aria-checked={o.id === p.value}
          tabIndex={o.id === p.value ? 0 : -1}
          onClick={() => p.onChange(o.id)}
        >
          {o.swatch && <span className="seg-swatch" style={{ background: o.swatch }} aria-hidden="true" />}
          {o.label}
        </button>
      ))}
    </div>
  )
}
