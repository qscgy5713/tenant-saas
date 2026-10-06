import { useQueryClient } from '@tanstack/react-query'
import { useMemo, useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { Loading, ErrorState, Notice } from '../../components/States'
import { ChevronLeftIcon, ChevronRightIcon, PlusIcon } from '../../components/Icons'
import { addDays, dayRange, formatDayLong } from '../../lib/time'
import { useToday } from '../../lib/useToday'
import { BookingRow } from '../components/BookingRow'
import { NewBookingModal } from '../components/NewBookingModal'
import { MonthCalendar } from '../components/MonthCalendar'
import { MoveConfirm, type Move } from '../components/MoveConfirm'
import { WeekCalendar } from '../components/WeekCalendar'
import { Modal } from '../components/Modal'
import { addMonths, monthWeeks, weekDays } from '../../lib/calendar'
import { bookingsKey, useAllBookings, useBookings, useMembers } from '../queries'
import { useShop } from '../ShopContext'

type View = 'day' | 'week' | 'month' | 'pending' | 'upcoming'

const VIEWS: { value: View; label: string }[] = [
  { value: 'day', label: '依日期' },
  { value: 'week', label: '週日曆' },
  { value: 'month', label: '月曆' },
  { value: 'pending', label: '待確認' },
  { value: 'upcoming', label: '近 30 天' },
]

const LIMIT = 100
/** 日曆一次最多抓的筆數(與 useAllBookings 的頁數上限一致) */
const GRID_MAX = 500

/** 各檢視「前一個 / 後一個」的步進與無障礙標籤 */
const STEP: Record<string, { prev: string; next: string }> = {
  day: { prev: '前一天', next: '後一天' },
  week: { prev: '上一週', next: '下一週' },
  month: { prev: '上個月', next: '下個月' },
}
function step(view: string, date: string, dir: 1 | -1): string {
  if (view === 'month') return addMonths(date, dir)
  return addDays(date, (view === 'week' ? 7 : 1) * dir)
}
const isYmd = (v: string | null): v is string => !!v && /^\d{4}-\d{2}-\d{2}$/.test(v)

export function BookingsPage() {
  const { slug, shop, canManage } = useShop()
  const queryClient = useQueryClient()
  const [params, setParams] = useSearchParams()
  const [creating, setCreating] = useState(false)
  const [flash, setFlash] = useState<string | null>(null)
  const [pickedId, setPickedId] = useState<string | null>(null)
  const [move, setMove] = useState<Move | null>(null)
  const tz = shop.timezone
  // 「現在」在同一天內固定(避免查詢條件每次渲染都變、一直重新請求),跨過午夜才會更新
  const { today, now } = useToday(tz)
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

  const isGrid = view === 'week' || view === 'month'
  const bookings = useBookings(slug, query, !isGrid)

  const gridQuery = useMemo(() => {
    const days = view === 'month' ? monthWeeks(date).flat() : weekDays(date)
    return {
      from: dayRange(days[0], tz).from,
      to: dayRange(days[days.length - 1], tz).to,
      staffId,
    }
  }, [view, date, tz, staffId])
  const grid = useAllBookings(slug, gridQuery, isGrid)
  // 以 id 從最新資料找,而不是存整筆:改期 / 取消之後 modal 內容才會跟著更新
  const picked = grid.data?.items.find((b) => b.id === pickedId) ?? null

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

        {(view === 'day' || isGrid) && (
          <div className="date-nav">
            <button
              type="button"
              className="icon-btn icon-btn-sm"
              aria-label={STEP[view]?.prev}
              onClick={() => update({ date: step(view, date, -1) })}
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
              aria-label={STEP[view]?.next}
              onClick={() => update({ date: step(view, date, 1) })}
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
                {m.active ? m.name : `${m.name}(已停用)`}
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

      {isGrid && (
        <>
          {view === 'month' && (
            <h2 className="day-title">
              {date.slice(0, 4)}年{Number(date.slice(5, 7))}月
            </h2>
          )}
          {grid.isPending && <Loading label="載入預約…" />}
          {grid.isError && <ErrorState error={grid.error} onRetry={() => grid.refetch()} />}
          {grid.data && view === 'week' && (
            <WeekCalendar
              date={date}
              tz={tz}
              bookings={grid.data.items}
              onPick={(b) => setPickedId(b.id)}
              // 與詳情裡「改期」按鈕的條件相同:已確認、還沒開始
              movable={(b) => b.status === 'confirmed' && new Date(b.starts_at) > new Date()}
              onMove={(booking, start) => setMove({ booking, start })}
            />
          )}
          {grid.data && view === 'month' && (
            <MonthCalendar
              date={date}
              tz={tz}
              bookings={grid.data.items}
              onPick={(b) => setPickedId(b.id)}
              onOpenDay={(day) => update({ view: null, date: day })}
            />
          )}
          {grid.data?.truncated && (
            <Notice tone="info">
              這段期間的預約太多,只顯示前 {GRID_MAX} 筆,請改用服務人員縮小範圍。
            </Notice>
          )}
          <Modal open={!!picked} title="預約詳情" onClose={() => setPickedId(null)} size="lg">
            {picked && (
              <ul className="rows">
                <BookingRow booking={picked} showDate />
              </ul>
            )}
          </Modal>
        </>
      )}

      {!isGrid && bookings.isPending && <Loading label="載入預約…" />}
      {!isGrid && bookings.isError && (
        <ErrorState error={bookings.error} onRetry={() => bookings.refetch()} />
      )}

      {!isGrid && bookings.data && bookings.data.items.length === 0 && (
        <div className="empty-card">
          <p>
            {view === 'day' && '這一天沒有預約。'}
            {view === 'pending' && '沒有待確認的預約。'}
            {view === 'upcoming' && '接下來 30 天沒有已確認的預約。'}
          </p>
        </div>
      )}

      {!isGrid && bookings.data && bookings.data.items.length > 0 && (
        <ul className="rows">
          {bookings.data.items.map((b) => (
            <BookingRow key={b.id} booking={b} showDate={view !== 'day'} />
          ))}
        </ul>
      )}

      {!isGrid && bookings.data && bookings.data.items.length >= LIMIT && (
        <Notice tone="info">只顯示前 {LIMIT} 筆,請用日期或服務人員縮小範圍。</Notice>
      )}

      <MoveConfirm
        key={move ? `${move.booking.id}@${move.start}` : 'none'}
        move={move}
        onClose={() => setMove(null)}
        onDone={setFlash}
      />
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
