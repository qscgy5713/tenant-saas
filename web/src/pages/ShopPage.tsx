import { Link, useParams } from 'react-router-dom'
import { useServices, useShop } from '../api/queries'
import { ErrorState, Loading } from '../components/States'
import { Avatar } from '../components/Avatar'
import { ArrowRightIcon, ClockIcon } from '../components/Icons'
import { Shell } from '../components/Shell'
import { formatDuration, formatPrice } from '../lib/money'
import { telHref } from '../lib/phone'
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
      <section className="hero">
        <Avatar name={shop.data.name} size="lg" />
        <div>
          <h1>{shop.data.name}</h1>
          {shop.data.description && <p className="hero-desc">{shop.data.description}</p>}
          {(shop.data.address || shop.data.phone) && (
            <ul className="shop-contact" aria-label="店家聯絡資訊">
              {shop.data.address && (
                <li>
                  <span className="muted">地址</span>{' '}
                  <a
                    href={`https://www.google.com/maps/search/?api=1&query=${encodeURIComponent(shop.data.address)}`}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {shop.data.address}
                  </a>
                </li>
              )}
              {shop.data.phone && (
                <li>
                  <span className="muted">電話</span>{' '}
                  <a href={telHref(shop.data.phone)}>{shop.data.phone}</a>
                </li>
              )}
            </ul>
          )}
          <p className="hero-sub">選擇服務,挑一個方便的時間,幾個步驟就完成預約。</p>
          <ul className="hero-chips" aria-label="預約說明">
            <li>不用註冊</li>
            <li>Email 確認</li>
            <li>可線上改期</li>
          </ul>
        </div>
      </section>

      <h2 className="section-title">選擇服務</h2>
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
              <div className="service-main">
                <h3>{service.name}</h3>
                <p className="service-meta">
                  <ClockIcon width={15} height={15} />
                  {formatDuration(service.duration_minutes)}
                </p>
                <p className="service-price">{formatPrice(service.price_cents)}</p>
              </div>
              <span className="btn btn-pill" aria-hidden="true">
                預約
                <ArrowRightIcon width={16} height={16} />
              </span>
            </Link>
          </li>
        ))}
      </ul>
    </Shell>
  )
}
