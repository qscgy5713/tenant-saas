import { useQueryClient } from '@tanstack/react-query'
import { useMemo, useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { Loading, ErrorState, Notice } from '../../components/States'
import { ChevronLeftIcon, ChevronRightIcon, PlusIcon } from '../../components/Icons'
import { addDays, dayRange, formatDayLong, todayYmd } from '../../lib/time'
import { BookingRow } from '../components/BookingRow'
import { NewBookingModal } from '../components/NewBookingModal'
import { bookingsKey, useBookings, useMembers } from '../queries'
import { useShop } from '../ShopContext'

type View = 'day' | 'pending' | 'upcoming'

const VIEWS: { value: View; label: string }[] = [
  { value: 'day', label: '依日期' },
  { value: 'pending', label: '待確認' },
  { value: 'upcoming', label: '近 30 天' },
]

const LIMIT = 100
const isYmd = (v: string | null): v is string => !!v && /^\d{4}-\d{2}-\d{2}$/.test(v)

export function BookingsPage() {
  const { slug, shop, canManage } = useShop()
  const queryClient = useQueryClient()
  const [params, setParams] = useSearchParams()
  const [creating, setCreating] = useState(false)
  const [flash, setFlash] = useState<string | null>(null)
  // 「現在」在這個頁面停留期間固定,避免查詢條件每次渲染都變、一直重新請求
  const [now] = useState(() => new Date())

  const tz = shop.timezone
  const today = todayYmd(tz, now)
  const view: View = (VIEWS.find((v) => v.value === params.get('view'))?.value ?? 'day') as View
  const dateParam = params.get('date')
  const date = isYmd(dateParam) ? dateParam : today
  const staffId = canManage ? (params.get('staff') ?? undefined) : undefined

  const members = useMembers(slug)

  const query = useMemo(() => {
    if (view === 'day') return { ...dayRange(date, tz), staffId, limit: LIMIT }
    if (view === 'pending')
      return { status: 'pending' as const, from: now.toISOString(), staffId, limit: LIMIT }
    return {
      status: 'confirmed' as const,
      from: now.toISOString(),
      to: new Date(now.getTime() + 30 * 86_400_000).toISOString(),
      staffId,
      limit: LIMIT,
    }
  }, [view, date, tz, staffId, now])

  const bookings = useBookings(slug, query)

  const update = (patch: Record<string, string | null>) => {
    const next = new URLSearchParams(params)
    for (const [k, v] of Object.entries(patch)) {
      if (v) next.set(k, v)
      else next.delete(k)
    }
    setParams(next, { replace: true })
  }

  return (
    <div>
      <div className="page-head">
        <div>
          <h1 className="page-heading">預約</h1>
          <p className="muted small">時間皆為店家當地時間</p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn btn-primary" onClick={() => setCreating(true)}>
            <PlusIcon width={16} height={16} />
            新增預約
          </button>
        </div>
      </div>

      {flash && <Notice tone="success">{flash}</Notice>}

      <div className="toolbar">
        <div className="segmented" role="group" aria-label="檢視">
          {VIEWS.map((v) => (
            <button
              key={v.value}
              type="button"
              aria-pressed={view === v.value}
              onClick={() => update({ view: v.value === 'day' ? null : v.value })}
            >
              {v.label}
            </button>
          ))}
        </div>

        {view === 'day' && (
          <div className="date-nav">
            <button
              type="button"
              className="icon-btn icon-btn-sm"
              aria-label="前一天"
              onClick={() => update({ date: addDays(date, -1) })}
            >
              <ChevronLeftIcon width={18} height={18} />
            </button>
            <input
              type="date"
              aria-label="日期"
              value={date}
              onChange={(e) => isYmd(e.target.value) && update({ date: e.target.value })}
            />
            <button
              type="button"
              className="icon-btn icon-btn-sm"
              aria-label="後一天"
              onClick={() => update({ date: addDays(date, 1) })}
            >
              <ChevronRightIcon width={18} height={18} />
            </button>
            {date !== today && (
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={() => update({ date: null })}
              >
                今天
              </button>
            )}
          </div>
        )}

        {canManage && (members.data?.length ?? 0) > 1 && (
          <select
            aria-label="服務人員"
            value={staffId ?? ''}
            onChange={(e) => update({ staff: e.target.value || null })}
          >
            <option value="">全部人員</option>
            {members.data?.map((m) => (
              <option key={m.user_id} value={m.user_id}>
                {m.name}
              </option>
            ))}
          </select>
        )}
      </div>

      {view === 'day' && (
        <h2 className="day-title">
          {formatDayLong(date)}
          {date === today ? '(今天)' : ''}
        </h2>
      )}

      {bookings.isPending && <Loading label="載入預約…" />}
      {bookings.isError && <ErrorState error={bookings.error} onRetry={() => bookings.refetch()} />}

      {bookings.data && bookings.data.items.length === 0 && (
        <div className="empty-card">
          <p>
            {view === 'day' && '這一天沒有預約。'}
            {view === 'pending' && '沒有待確認的預約。'}
            {view === 'upcoming' && '接下來 30 天沒有已確認的預約。'}
          </p>
        </div>
      )}

      {bookings.data && bookings.data.items.length > 0 && (
        <ul className="rows">
          {bookings.data.items.map((b) => (
            <BookingRow key={b.id} booking={b} showDate={view !== 'day'} />
          ))}
        </ul>
      )}

      {bookings.data && bookings.data.items.length >= LIMIT && (
        <Notice tone="info">只顯示前 {LIMIT} 筆,請用日期或服務人員縮小範圍。</Notice>
      )}

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
