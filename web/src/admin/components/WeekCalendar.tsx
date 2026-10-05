import { useMemo, useRef, useState, type DragEvent } from 'react'
import {
  UNITS_PER_HOUR,
  hourWindow,
  layoutDay,
  startAtUnit,
  unitFromTop,
  weekDays,
  type PlacedBlock,
} from '../../lib/calendar'
import { formatTime, todayYmd, weekdayShort, dayOfMonth } from '../../lib/time'
import { STATUS_LABEL } from '../labels'
import type { AdminBooking } from '../types'

/**
 * 週日曆(週一開始)。已取消的預約不畫(時段已釋出),想看請用列表檢視。
 * `movable` 的預約可以拖到別的日期 / 時間(吸附到 15 分鐘),放開時呼叫 `onMove`,由上層確認並送出。
 * 拖曳只是捷徑:滑鼠限定,鍵盤與觸控請用詳情裡的「改期」。
 */
export function WeekCalendar({
  date,
  tz,
  bookings,
  onPick,
  movable,
  onMove,
}: {
  date: string
  tz: string
  bookings: AdminBooking[]
  onPick: (booking: AdminBooking) => void
  movable?: (booking: AdminBooking) => boolean
  onMove?: (booking: AdminBooking, start: string) => void
}) {
  // 拖曳中的預約與抓取點(抓在區塊的哪個高度,放開時以區塊上緣對齊格線)
  const drag = useRef<{ booking: AdminBooking; grabPx: number } | null>(null)
  const [draggingId, setDraggingId] = useState<string | null>(null)
  const [overDay, setOverDay] = useState<string | null>(null)

  const days = useMemo(() => weekDays(date), [date])
  const today = todayYmd(tz)
  const shown = useMemo(() => bookings.filter((b) => b.status !== 'cancelled'), [bookings])
  const [startHour, endHour] = useMemo(() => hourWindow(shown, days, tz), [shown, days, tz])
  const hours = endHour - startHour

  const columns = useMemo(
    () => days.map((day) => ({ day, blocks: layoutDay(shown, day, tz, startHour) })),
    [days, shown, tz, startHour],
  )

  const endDrag = () => {
    drag.current = null
    setDraggingId(null)
    setOverDay(null)
  }

  const drop = (e: DragEvent<HTMLDivElement>, day: string) => {
    const d = drag.current
    if (!d || !onMove) return
    e.preventDefault()
    const rect = e.currentTarget.getBoundingClientRect()
    const unit = unitFromTop(e.clientY - rect.top - d.grabPx, rect.height, hours * UNITS_PER_HOUR)
    const start = startAtUnit(day, startHour, unit, tz)
    const booking = d.booking
    endDrag()
    // 放回原位 = 沒有改動
    if (new Date(start).getTime() !== new Date(booking.starts_at).getTime()) onMove(booking, start)
  }

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
            className={`cal-col cal-hours-${hours} ${day === today ? 'cal-col-today' : ''} ${overDay === day ? 'cal-col-over' : ''}`}
            onDragOver={(e) => {
              if (!drag.current) return
              e.preventDefault()
              e.dataTransfer.dropEffect = 'move'
              if (overDay !== day) setOverDay(day)
            }}
            onDrop={(e) => drop(e, day)}
          >
            {blocks.map((b) => (
              <Block
                key={b.item.id}
                block={b}
                tz={tz}
                onPick={onPick}
                dragging={draggingId === b.item.id}
                draggable={!!onMove && !!movable?.(b.item)}
                onDragStart={(e) => {
                  const rect = e.currentTarget.getBoundingClientRect()
                  drag.current = { booking: b.item, grabPx: e.clientY - rect.top }
                  setDraggingId(b.item.id)
                  e.dataTransfer.effectAllowed = 'move'
                  // Firefox 沒有 setData 就不會開始拖曳
                  e.dataTransfer.setData('text/plain', b.item.id)
                }}
                onDragEnd={endDrag}
              />
            ))}
          </div>
        ))}
      </div>
    </div>
  )
}

/** 外層負責位置與拖曳(Firefox 不能直接拖 <button>),按鈕負責點擊與鍵盤 */
function Block({
  block,
  tz,
  onPick,
  dragging,
  draggable,
  onDragStart,
  onDragEnd,
}: {
  block: PlacedBlock<AdminBooking>
  tz: string
  onPick: (booking: AdminBooking) => void
  dragging: boolean
  draggable: boolean
  onDragStart: (e: DragEvent<HTMLDivElement>) => void
  onDragEnd: () => void
}) {
  const b = block.item
  return (
    <div
      className={`cal-item cal-top-${Math.min(95, block.top)} cal-len-${Math.min(96, block.len)} cal-lane-${block.lane}-of-${block.lanes} ${draggable ? 'cal-item-movable' : ''} ${dragging ? 'cal-item-dragging' : ''}`}
      draggable={draggable}
      onDragStart={draggable ? onDragStart : undefined}
      onDragEnd={draggable ? onDragEnd : undefined}
    >
      <button
        type="button"
        className={`cal-block cal-block-${b.status}`}
        aria-label={`${formatTime(b.starts_at, tz)} ${b.customer_name} ${b.service_name}(${STATUS_LABEL[b.status]})`}
        onClick={() => onPick(b)}
      >
        <span className="cal-block-time">{formatTime(b.starts_at, tz)}</span> {b.customer_name}
        <br />
        {b.service_name}
      </button>
    </div>
  )
}
