import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router'
import Hero, { HeroSkeleton } from '../components/Hero'
import PosterCard from '../components/PosterCard'
import Row from '../components/Row'
import { CardSkeletons, ErrorState } from '../components/States'
import { api, type Home as HomeData, type Item } from '../lib/api'

const ROWS: { key: keyof HomeData; title: string; more?: string }[] = [
  { key: 'trending', title: 'Trending' },
  { key: 'popular_movies', title: 'Beliebte Filme', more: '/filme' },
  { key: 'popular_tv', title: 'Beliebte Serien', more: '/serien' },
  { key: 'top_rated_movies', title: 'Top bewertete Filme', more: '/filme?liste=top_rated' },
  { key: 'top_rated_tv', title: 'Top bewertete Serien', more: '/serien?liste=top_rated' },
]

export default function Home() {
  const { data, isPending, error, refetch } = useQuery({ queryKey: ['home'], queryFn: api.home })

  if (error) return <ErrorState error={error} onRetry={() => void refetch()} />

  return (
    <div className="home">
      {isPending ? <HeroSkeleton /> : <Hero items={data.trending} />}
      <div className="shelves">
        {ROWS.map((r) => (
          <Row
            key={r.key}
            title={r.title}
            action={
              r.more ? (
                <Link to={r.more} className="row-more">
                  Alle anzeigen
                </Link>
              ) : undefined
            }
          >
            {isPending ? (
              <CardSkeletons count={8} />
            ) : (
              data[r.key].map((item: Item) => <PosterCard key={`${item.type}-${item.id}`} item={item} showType={r.key === 'trending'} />)
            )}
          </Row>
        ))}
      </div>
    </div>
  )
}
