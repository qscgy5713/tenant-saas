import { useMemo } from 'react'
import {
  UNITS_PER_HOUR,
  hourWindow,
  layoutDay,
  weekDays,
  type PlacedBlock,
} from '../../lib/calendar'
import { formatTime, todayYmd, weekdayShort, dayOfMonth } from '../../lib/time'
import { STATUS_LABEL } from '../labels'
import type { AdminBooking } from '../types'

/** 週日曆(週一開始)。已取消的預約不畫(時段已釋出),想看請用列表檢視 */
export function WeekCalendar({
  date,
  tz,
  bookings,
  onPick,
}: {
  date: string
  tz: string
  bookings: AdminBooking[]
  onPick: (booking: AdminBooking) => void
}) {
  const days = useMemo(() => weekDays(date), [date])
  const today = todayYmd(tz)
  const shown = useMemo(() => bookings.filter((b) => b.status !== 'cancelled'), [bookings])
  const [startHour, endHour] = useMemo(() => hourWindow(shown, days, tz), [shown, days, tz])
  const hours = endHour - startHour

  const columns = useMemo(
    () => days.map((day) => ({ day, blocks: layoutDay(shown, day, tz, startHour) })),
    [days, shown, tz, startHour],
  )

  return (
    <div className="cal" role="region" aria-label="週日曆">
      <div className="cal-grid">
        <div className="cal-head" />
        {days.map((day) => (
          <div key={day} className={`cal-head ${day === today ? 'cal-head-today' : ''}`}>
            {weekdayShort(day)} {dayOfMonth(day)}
            {day === today && <span className="sr-only">(今天)</span>}
          </div>
        ))}

        <div className={`cal-hours cal-hours-${hours}`} aria-hidden="true">
          {Array.from({ length: hours }, (_, i) => (
            <span key={i} className={`cal-hour-label cal-top-${i * UNITS_PER_HOUR}`}>
              {i === 0 ? '' : `${String(startHour + i).padStart(2, '0')}:00`}
            </span>
          ))}
        </div>
        {columns.map(({ day, blocks }) => (
          <div
            key={day}
            className={`cal-col cal-hours-${hours} ${day === today ? 'cal-col-today' : ''}`}
          >
            {blocks.map((b) => (
              <Block key={b.item.id} block={b} tz={tz} onPick={onPick} />
            ))}
          </div>
        ))}
      </div>
    </div>
  )
}

function Block({
  block,
  tz,
  onPick,
}: {
  block: PlacedBlock<AdminBooking>
  tz: string
  onPick: (booking: AdminBooking) => void
}) {
  const b = block.item
  return (
    <button
      type="button"
      className={`cal-block cal-block-${b.status} cal-top-${Math.min(95, block.top)} cal-len-${Math.min(96, block.len)} cal-lane-${block.lane}-of-${block.lanes}`}
      aria-label={`${formatTime(b.starts_at, tz)} ${b.customer_name} ${b.service_name}(${STATUS_LABEL[b.status]})`}
      onClick={() => onPick(b)}
    >
      <span className="cal-block-time">{formatTime(b.starts_at, tz)}</span> {b.customer_name}
      <br />
      {b.service_name}
    </button>
  )
}
