export type IconName =
  | 'play'
  | 'pause'
  | 'next'
  | 'plus'
  | 'check'
  | 'search'
  | 'star'
  | 'left'
  | 'right'
  | 'down'
  | 'close'
  | 'film'
  | 'tv'
  | 'home'
  | 'bookmark'
  | 'refresh'
  | 'volume'
  | 'mute'
  | 'expand'
  | 'shrink'
  | 'sliders'
  | 'subtitles'
  | 'audio'
  | 'layers'
  | 'info'
  | 'bolt'
  | 'warn'
  | 'app'
  | 'clock'
  | 'trash'
  | 'download'
  | 'shield'
  | 'folder'
  | 'seed'

const paths: Record<IconName, string> = {
  play: 'M7 4.5v15a1 1 0 0 0 1.5.86l12.5-7.5a1 1 0 0 0 0-1.72L8.5 3.64A1 1 0 0 0 7 4.5Z',
  pause: 'M7 4.5h3.5v15H7zM13.5 4.5H17v15h-3.5z',
  next: 'M5 5.2v13.6a.8.8 0 0 0 1.2.7l10-6.8a.8.8 0 0 0 0-1.4l-10-6.8a.8.8 0 0 0-1.2.7ZM18 4.5h2v15h-2z',
  plus: 'M12 5v14M5 12h14',
  check: 'm5 12.5 4.5 4.5L19 7.5',
  search: 'M10.5 18a7.5 7.5 0 1 1 0-15 7.5 7.5 0 0 1 0 15Zm5.3-2.2L21 21',
  star: 'm12 3.2 2.7 5.6 6.1.8-4.5 4.2 1.1 6.1L12 17l-5.4 2.9 1.1-6.1-4.5-4.2 6.1-.8Z',
  left: 'm15 5-7 7 7 7',
  right: 'm9 5 7 7-7 7',
  down: 'm5 9 7 7 7-7',
  close: 'M6 6l12 12M18 6 6 18',
  film: 'M4 5h16v14H4zM8 5v14M16 5v14M4 9.5h4M4 14.5h4M16 9.5h4M16 14.5h4',
  tv: 'M3.5 6.5h17v11h-17zM8 20.5h8',
  home: 'M4 10.8 12 4l8 6.8V19a1 1 0 0 1-1 1h-4.5v-5.5h-5V20H5a1 1 0 0 1-1-1Z',
  bookmark: 'M6.5 3.5h11v17L12 16.5l-5.5 4Z',
  refresh: 'M20 11a8 8 0 1 0-2.3 5.7M20 4.5V11h-6.5',
  volume: 'M4 9.5h3.5L12 5.5v13l-4.5-4H4zM15.5 9a4.2 4.2 0 0 1 0 6M18 6.5a7.8 7.8 0 0 1 0 11',
  mute: 'M4 9.5h3.5L12 5.5v13l-4.5-4H4zM16 9.5l5 5M21 9.5l-5 5',
  expand: 'M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5',
  shrink: 'M9 4v5H4M20 9h-5V4M15 20v-5h5M4 15h5v5',
  sliders: 'M4 7h9M17 7h3M4 17h3M11 17h9M15 4.5v5M9 14.5v5',
  subtitles: 'M3.5 6h17v12h-17zM7 11h3M13 11h4M7 14.5h6M16 14.5h1',
  audio: 'M4 10v4M8 6.5v11M12 9v6M16 5v14M20 10v4',
  layers: 'M12 4 3.5 8.5 12 13l8.5-4.5ZM3.5 12.5 12 17l8.5-4.5M3.5 16.5 12 21l8.5-4.5',
  info: 'M12 21a9 9 0 1 1 0-18 9 9 0 0 1 0 18ZM12 11v5.5M12 7.6v.2',
  bolt: 'M13 3 5 13.5h6L10.5 21 19 10h-6Z',
  warn: 'M12 4 2.8 19.5h18.4ZM12 10v4.5M12 17.3v.2',
  app: 'M3.5 5h17v11h-17zM9 20h6M12 16v4',
  clock: 'M12 21a9 9 0 1 1 0-18 9 9 0 0 1 0 18ZM12 7.5V12l3 2',
  trash: 'M4.5 7h15M9.5 7V4.5h5V7M6.5 7l1 12.5h9l1-12.5M10 11v5M14 11v5',
  download: 'M12 4v11M7 10.5l5 5 5-5M5 19.5h14',
  shield: 'M12 3.5 5 6v5.5c0 4.3 3 7.6 7 9 4-1.4 7-4.7 7-9V6ZM9 12l2.2 2.2L15.5 10',
  folder: 'M3.5 7a1.5 1.5 0 0 1 1.5-1.5h4.5l2 2.5H19a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 19H5a1.5 1.5 0 0 1-1.5-1.5Z',
  seed: 'M12 19V6M7 10.5l5-5 5 5',
}

const filled: Partial<Record<IconName, true>> = { play: true, pause: true, next: true, star: true }

type Props = { name: IconName; size?: number; solid?: boolean; stroke?: number }

/** One outline set on a 24 grid; `currentColor` everywhere so states recolor through CSS. */
export default function Icon({ name, size = 20, solid, stroke = 1.9 }: Props) {
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
      strokeWidth={stroke}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d={paths[name]} />
    </svg>
  )
}
