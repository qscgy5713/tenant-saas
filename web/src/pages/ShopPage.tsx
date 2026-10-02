import { Link, useParams } from 'react-router-dom'
import { useServices, useShop } from '../api/queries'
import { ErrorState, Loading } from '../components/States'
import { Shell } from '../components/Shell'
import { formatDuration, formatPrice } from '../lib/money'
import { ApiError } from '../api/client'

export function ShopPage() {
  const { slug = '' } = useParams()
  const shop = useShop(slug)
  const services = useServices(slug)

  if (shop.isPending)
    return (
      <Shell>
        <Loading />
      </Shell>
    )
  if (shop.isError) {
    const notFound = shop.error instanceof ApiError && shop.error.status === 404
    return (
      <Shell title={notFound ? '找不到店家' : '發生問題'}>
        <ErrorState
          error={shop.error}
          title={notFound ? '找不到這家店' : '發生問題'}
          onRetry={() => shop.refetch()}
        >
          {notFound && <p className="muted">請確認預約連結是否正確。</p>}
        </ErrorState>
      </Shell>
    )
  }

  return (
    <Shell title="選擇服務" shopName={shop.data.name} shopSlug={slug}>
      <h1 className="page-title">選擇服務</h1>
      {services.isPending && <Loading label="載入服務項目…" />}
      {services.isError && <ErrorState error={services.error} onRetry={() => services.refetch()} />}
      {services.data?.length === 0 && (
        <div className="state">
          <p>目前沒有開放預約的服務。</p>
        </div>
      )}
      <ul className="cards">
        {services.data?.map((service) => (
          <li key={service.id}>
            <Link to={`/s/${slug}/book/${service.id}`} className="card service-card">
              <div>
                <h2>{service.name}</h2>
                <p className="muted">
                  {formatDuration(service.duration_minutes)} · {formatPrice(service.price_cents)}
                </p>
              </div>
              <span className="btn btn-secondary" aria-hidden="true">
                預約
              </span>
            </Link>
          </li>
        ))}
      </ul>
    </Shell>
  )
}
