import { useMemo } from 'react'
import { groupByLocalDay, monthStart, monthWeeks } from '../../lib/calendar'
import { dayOfMonth, formatTime, weekdayShort } from '../../lib/time'
import { useToday } from '../../lib/useToday'
import { STATUS_LABEL } from '../labels'
import type { AdminBooking } from '../types'

/** 每個日期格最多列出幾筆,其餘用「還有 N 筆」連到當天的列表 */
const MAX_CHIPS = 3
const WEEKDAY_HEADERS = [
  '2026-10-05',
  '2026-10-06',
  '2026-10-07',
  '2026-10-08',
  '2026-10-09',
  '2026-10-10',
  '2026-10-11',
]

/** 月曆(週一開始)。已取消的不畫;點日期進入當天的列表,點預約開啟詳情 */
export function MonthCalendar({
  date,
  tz,
  bookings,
  onPick,
  onOpenDay,
}: {
  date: string
  tz: string
  bookings: AdminBooking[]
  onPick: (booking: AdminBooking) => void
  onOpenDay: (day: string) => void
}) {
  const weeks = useMemo(() => monthWeeks(date), [date])
  const { today } = useToday(tz)
  const month = monthStart(date).slice(0, 7)
  const byDay = useMemo(
    () =>
      groupByLocalDay(
        bookings.filter((b) => b.status !== 'cancelled'),
        tz,
      ),
    [bookings, tz],
  )

  return (
    <div className="cal" role="region" aria-label="月曆">
      <div className="month-grid">
        {WEEKDAY_HEADERS.map((d) => (
          <div key={d} className="cal-head">
            {weekdayShort(d)}
          </div>
        ))}
        {weeks.flat().map((day) => {
          const items = byDay.get(day) ?? []
          const inMonth = day.startsWith(month)
          return (
            <div
              key={day}
              className={`month-cell ${inMonth ? '' : 'month-cell-out'} ${day === today ? 'month-cell-today' : ''}`}
            >
              <button
                type="button"
                className="month-day"
                aria-label={`${day}${items.length ? `,${items.length} 筆預約` : ''}`}
                onClick={() => onOpenDay(day)}
              >
                {dayOfMonth(day)}
              </button>
              {items.slice(0, MAX_CHIPS).map((b) => (
                <button
                  key={b.id}
                  type="button"
                  className={`month-chip cal-block-${b.status}`}
                  aria-label={`${formatTime(b.starts_at, tz)} ${b.customer_name}(${STATUS_LABEL[b.status]})`}
                  onClick={() => onPick(b)}
                >
                  {formatTime(b.starts_at, tz)} {b.customer_name}
                </button>
              ))}
              {items.length > MAX_CHIPS && (
                <button type="button" className="month-more" onClick={() => onOpenDay(day)}>
                  還有 {items.length - MAX_CHIPS} 筆
                </button>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}
