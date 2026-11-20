import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router'
import Icon from '../components/Icon'
import Hero, { HeroSkeleton } from '../components/Hero'
import PosterCard, { WideCard, WideSkeletons } from '../components/PosterCard'
import Row from '../components/Row'
import { CardSkeletons, ErrorState } from '../components/States'
import { ContinueCard, NewEpisodeCard } from '../components/WatchCards'
import { useI18n, type Key } from '../i18n'
import { api, type Home as HomeData, type Item } from '../lib/api'
import { useContinue, useHideContinue, useNewEpisodes } from '../lib/progress'

const ROWS: { key: keyof HomeData; title: Key; more?: string }[] = [
  { key: 'trending', title: 'home.trending' },
  { key: 'popular_movies', title: 'home.popularMovies', more: '/filme' },
  { key: 'popular_tv', title: 'home.popularTv', more: '/serien' },
  { key: 'top_rated_movies', title: 'home.topMovies', more: '/filme?liste=top_rated' },
  { key: 'top_rated_tv', title: 'home.topTv', more: '/serien?liste=top_rated' },
]

export default function Home() {
  const { t } = useI18n()
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
          <Row title={t('home.continue')}>
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
          <Row title={t('home.newEpisodes')}>
            {fresh.data.map((n) => (
              <NewEpisodeCard key={n.item.id} ep={n} />
            ))}
          </Row>
        )}
        {ROWS.map((r) => (
          <Row
            key={r.key}
            title={t(r.title)}
            action={
              r.more ? (
                <Link to={r.more} className="row-more">
                  {t('home.all')}
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
