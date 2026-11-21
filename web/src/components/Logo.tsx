import { Link } from 'react-router'
import { useI18n } from '../i18n'
import { MARK, WORD, WORD_CAP, WORD_DOT, WORD_WIDTH } from './brand'

const CAP = 118
const K = CAP / WORD_CAP
const WORD_X = 262
const WIDTH = Math.round(WORD_X + WORD_WIDTH * K - 48)

/** The symbol alone: an F whose arms flicker out into thinner strips. */
export function Mark({ size = 28, className }: { size?: number; className?: string }) {
  return (
    <svg className={className} width={size} height={size} viewBox="40 28 192 200" aria-hidden="true" focusable="false">
      <path fill="currentColor" d={MARK} />
    </svg>
  )
}

/** Horizontal lockup, symbol plus wordmark. The i's tittle is the projector lamp. */
function Lockup({ height = 26 }: { height?: number }) {
  return (
    <svg
      className="lockup"
      height={height}
      width={(height * WIDTH) / 184}
      viewBox={`48 36 ${WIDTH} 184`}
      aria-hidden="true"
      focusable="false"
    >
      <path className="lockup-mark" d={MARK} />
      <g transform={`translate(${WORD_X} ${128 - CAP / 2}) scale(${K})`}>
        <path className="lockup-word" d={WORD} />
        <path className="lockup-lamp" d={WORD_DOT} />
      </g>
    </svg>
  )
}

export default function Logo() {
  const { t } = useI18n()
  return (
    <Link to="/" className="logo" aria-label={t('app.home')}>
      <Lockup />
    </Link>
  )
}
