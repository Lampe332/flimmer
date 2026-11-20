import type { Season } from '../lib/api'

type Props = { seasons: Season[]; value: number; onChange: (n: number) => void }

export default function SeasonTabs({ seasons, value, onChange }: Props) {
  return (
    <div className="tabs season-tabs" role="tablist" aria-label="Staffel wählen">
      {seasons.map((s) => (
        <button
          key={s.season_number}
          type="button"
          role="tab"
          className="tab"
          aria-selected={s.season_number === value}
          onClick={() => onChange(s.season_number)}
        >
          {s.name}
          <span className="tab-count">{s.episode_count}</span>
        </button>
      ))}
    </div>
  )
}
