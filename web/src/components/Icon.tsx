type Name =
  | 'play'
  | 'plus'
  | 'check'
  | 'search'
  | 'star'
  | 'left'
  | 'right'
  | 'close'
  | 'pause'
  | 'film'
  | 'shield'
  | 'bookmark'
  | 'refresh'

const paths: Record<Name, string> = {
  play: 'M7 4.5v15a1 1 0 0 0 1.5.86l12.5-7.5a1 1 0 0 0 0-1.72L8.5 3.64A1 1 0 0 0 7 4.5Z',
  plus: 'M12 5v14M5 12h14',
  check: 'm5 12.5 4.5 4.5L19 7.5',
  search: 'M10.5 18a7.5 7.5 0 1 1 0-15 7.5 7.5 0 0 1 0 15Zm5.3-2.2L21 21',
  star: 'm12 3.2 2.7 5.6 6.1.8-4.5 4.2 1.1 6.1L12 17l-5.4 2.9 1.1-6.1-4.5-4.2 6.1-.8Z',
  left: 'm15 5-7 7 7 7',
  right: 'm9 5 7 7-7 7',
  close: 'M6 6l12 12M18 6 6 18',
  pause: 'M8 5v14M16 5v14',
  film: 'M4 5h16v14H4zM8 5v14M16 5v14M4 9.5h4M4 14.5h4M16 9.5h4M16 14.5h4',
  shield: 'M12 3 4.5 6v5.5c0 4.6 3.2 8.4 7.5 9.5 4.3-1.1 7.5-4.9 7.5-9.5V6Z',
  bookmark: 'M6.5 3.5h11v17L12 16.5l-5.5 4Z',
  refresh: 'M20 11a8 8 0 1 0-2.3 5.7M20 4.5V11h-6.5',
}

const filled: Partial<Record<Name, true>> = { play: true, star: true }

export default function Icon({ name, size = 20, solid }: { name: Name; size?: number; solid?: boolean }) {
  const fill = filled[name] || solid
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      aria-hidden="true"
      focusable="false"
      fill={fill ? 'currentColor' : 'none'}
      stroke={fill ? 'none' : 'currentColor'}
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d={paths[name]} />
    </svg>
  )
}
