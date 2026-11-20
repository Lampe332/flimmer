import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router'
import Icon from '../components/Icon'
import Hero, { HeroSkeleton } from '../components/Hero'
import PosterCard, { WideCard, WideSkeletons } from '../components/PosterCard'
import Row from '../components/Row'
import { CardSkeletons, ErrorState } from '../components/States'
import { ContinueCard, NewEpisodeCard } from '../components/WatchCards'
import { api, type Home as HomeData, type Item } from '../lib/api'
import { useContinue, useHideContinue, useNewEpisodes } from '../lib/progress'

const ROWS: { key: keyof HomeData; title: string; more?: string }[] = [
  { key: 'trending', title: 'Heute angesagt' },
  { key: 'popular_movies', title: 'Beliebte Filme', more: '/filme' },
  { key: 'popular_tv', title: 'Beliebte Serien', more: '/serien' },
  { key: 'top_rated_movies', title: 'Top bewertete Filme', more: '/filme?liste=top_rated' },
  { key: 'top_rated_tv', title: 'Top bewertete Serien', more: '/serien?liste=top_rated' },
]

export default function Home() {
  const { data, isPending, error, refetch } = useQuery({ queryKey: ['home'], queryFn: api.home })
  const watching = useContinue()
  const fresh = useNewEpisodes()
  const hide = useHideContinue()

  if (error) return <ErrorState error={error} onRetry={() => void refetch()} />

  return (
    <div className="home">
      {isPending ? <HeroSkeleton /> : <Hero items={data.trending} />}
      <div className="shelves">
        {watching.data && watching.data.length > 0 && (
          <Row title="Weiterschauen">
            {watching.data.map((c) => (
              <ContinueCard
                key={`${c.item.type}-${c.item.id}`}
                entry={c}
                onRemove={() => hide.mutate({ type: c.item.type, id: c.item.id })}
              />
            ))}
          </Row>
        )}
        {fresh.data && fresh.data.length > 0 && (
          <Row title="Neue Folgen deiner Serien">
            {fresh.data.map((n) => (
              <NewEpisodeCard key={n.item.id} ep={n} />
            ))}
          </Row>
        )}
        {ROWS.map((r) => (
          <Row
            key={r.key}
            title={r.title}
            action={
              r.more ? (
                <Link to={r.more} className="row-more">
                  Alle anzeigen
                  <Icon name="right" size={14} />
                </Link>
              ) : undefined
            }
          >
            {r.key === 'trending' ? (
              isPending ? (
                <WideSkeletons />
              ) : (
                data.trending.filter((item) => item.backdrop).map((item) => <WideCard key={`${item.type}-${item.id}`} item={item} />)
              )
            ) : isPending ? (
              <CardSkeletons count={8} />
            ) : (
              data[r.key].map((item: Item) => <PosterCard key={`${item.type}-${item.id}`} item={item} />)
            )}
          </Row>
        ))}
      </div>
    </div>
  )
}
