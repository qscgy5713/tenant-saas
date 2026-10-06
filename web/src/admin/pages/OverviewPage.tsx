import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { Link } from 'react-router-dom'
import { CopyIcon, ExternalLinkIcon, PlusIcon } from '../../components/Icons'
import { ErrorState, Loading, Notice } from '../../components/States'
import { dayRange, formatDayLong } from '../../lib/time'
import { useToday } from '../../lib/useToday'
import { getPlan } from '../api'
import { BookingRow } from '../components/BookingRow'
import { NewBookingModal } from '../components/NewBookingModal'
import { bookingsKey, useBookings } from '../queries'
import { useShop } from '../ShopContext'

export function OverviewPage() {
  const { slug, shop, user, canManage } = useShop()
  const queryClient = useQueryClient()
  const [creating, setCreating] = useState(false)
  const [flash, setFlash] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const tz = shop.timezone
  // 頁面整夜開著也會跟著換日(櫃檯的平板),不會隔天還顯示昨天的行程
  const { today, now } = useToday(tz)
  const range = dayRange(today, tz)

  const todays = useBookings(slug, { ...range, limit: 100 })
  const pending = useBookings(slug, { status: 'pending', from: now.toISOString(), limit: 100 })
  const plan = useQuery({
    queryKey: ['admin', 'plan', slug],
    queryFn: () => getPlan(slug),
    enabled: canManage,
    staleTime: 30_000,
  })

  const live = (todays.data?.items ?? []).filter(
    (b) => b.status === 'pending' || b.status === 'confirmed',
  )
  const shopUrl = `${window.location.origin}/s/${slug}`

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(shopUrl)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // 沒有剪貼簿權限時,使用者仍可從上方連結複製
    }
  }

  const hour = now.getHours()
  const greeting = hour < 12 ? '早安' : hour < 18 ? '午安' : '晚安'
  const usage = plan.data?.usage
  const limit = plan.data?.plan.max_bookings_per_month

  return (
    <div>
      <div className="page-head">
        <div>
          <h1 className="page-heading">
            {greeting},{user.name}
          </h1>
          <p className="muted">{formatDayLong(today)}</p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn btn-primary" onClick={() => setCreating(true)}>
            <PlusIcon width={16} height={16} />
            新增預約
          </button>
        </div>
      </div>

      {flash && <Notice tone="success">{flash}</Notice>}

      <div className="stat-grid">
        <Stat
          label="今天的預約"
          value={todays.isPending ? '…' : String(live.length)}
          sub="不含已取消"
        />
        <Stat
          label="待確認"
          value={
            pending.isPending
              ? '…'
              : pending.data && pending.data.items.length >= 100
                ? '99+'
                : String(pending.data?.items.length ?? 0)
          }
          sub="顧客還沒按確認連結"
          to={`bookings?view=pending`}
        />
        {canManage && (
          <Stat
            label="本月預約"
            value={usage ? String(usage.bookings_this_month) : '…'}
            sub={limit ? `方案上限 ${limit}` : '方案不限'}
            to="plan"
          />
        )}
        <Stat label="顧客預約頁" value="" sub={shopUrl.replace(/^https?:\/\//, '')} custom>
          <div className="share">
            <a href={shopUrl} target="_blank" rel="noreferrer" className="btn btn-secondary btn-sm">
              <ExternalLinkIcon width={14} height={14} />
              開啟
            </a>
            <button type="button" className="btn btn-secondary btn-sm" onClick={copy}>
              <CopyIcon width={14} height={14} />
              {copied ? '已複製' : '複製連結'}
            </button>
          </div>
        </Stat>
      </div>

      <section className="section">
        <h2>今天的行程</h2>
        {todays.isPending && <Loading label="載入今天的預約…" />}
        {todays.isError && <ErrorState error={todays.error} onRetry={() => todays.refetch()} />}
        {todays.data && todays.data.items.length === 0 && (
          <div className="empty-card">
            <p>今天沒有預約。</p>
            <Link className="btn btn-secondary btn-sm" to="bookings">
              查看其他日期
            </Link>
          </div>
        )}
        {todays.data && todays.data.items.length > 0 && (
          <ul className="rows">
            {todays.data.items.map((b) => (
              <BookingRow key={b.id} booking={b} />
            ))}
          </ul>
        )}
      </section>

      <NewBookingModal
        open={creating}
        onClose={() => setCreating(false)}
        onCreated={() => {
          setCreating(false)
          setFlash('已建立預約,並寄出確認信給顧客。')
          queryClient.invalidateQueries({ queryKey: bookingsKey(slug) })
          queryClient.invalidateQueries({ queryKey: ['admin', 'plan', slug] })
        }}
      />
    </div>
  )
}

function Stat({
  label,
  value,
  sub,
  to,
  custom,
  children,
}: {
  label: string
  value: string
  sub: string
  to?: string
  custom?: boolean
  children?: React.ReactNode
}) {
  const body = (
    <>
      <span className="stat-label">{label}</span>
      {!custom && <span className="stat-value">{value}</span>}
      <span className="stat-sub">{sub}</span>
      {children}
    </>
  )
  return to ? (
    <Link to={to} className="stat stat-link">
      {body}
    </Link>
  ) : (
    <div className="stat">{body}</div>
  )
}
