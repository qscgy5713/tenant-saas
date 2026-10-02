import { useMemo, useState } from 'react'
import { useAvailability } from '../api/queries'
import { firstAvailableDay, groupByDay, groupByPeriod, type TimeOption } from '../lib/slots'
import {
  PERIOD_LABEL,
  addDays,
  dayOfMonth,
  diffDays,
  formatDayLong,
  formatMonthSpan,
  formatTime,
  todayYmd,
  tzLabel,
  weekdayShort,
} from '../lib/time'
import { ChevronLeftIcon, ChevronRightIcon } from './Icons'
import { ErrorState, Loading } from './States'

/** 後端限制最遠可預約 90 天、單次查詢最多 14 天。一頁顯示 7 天,每兩頁共用一次查詢 */
const DAYS_PER_PAGE = 7
const PAGES_PER_WINDOW = 2
const MAX_PAGE = 12

interface Props {
  slug: string
  serviceId: string
  timezone: string
  /** null = 不指定員工 */
  staffId: string | null
  selectedStart: string | null
  onSelect: (option: TimeOption) => void
}

export function SlotPicker({ slug, serviceId, timezone, staffId, selectedStart, onSelect }: Props) {
  const today = todayYmd(timezone)
  // 使用者主動翻頁 / 點選日期之前,頁面與日期都由資料推導(預設跳到最早有空的那天)
  const [userPage, setUserPage] = useState<number | null>(null)
  const [userDay, setUserDay] = useState<string | null>(null)

  const windowOf = (page: number) => Math.floor(page / PAGES_PER_WINDOW)
  const windowFrom = (w: number) => addDays(today, w * DAYS_PER_PAGE * PAGES_PER_WINDOW)

  // 第一次載入時還不知道最早有空的是哪天,先抓第一個視窗
  const requestedPage = userPage ?? 0
  const fromDay = windowFrom(windowOf(requestedPage))
  const query = useAvailability({
    slug,
    serviceId,
    staffId,
    from: fromDay,
    to: addDays(fromDay, DAYS_PER_PAGE * PAGES_PER_WINDOW - 1),
  })

  const byDay = useMemo(() => groupByDay(query.data?.slots ?? [], timezone), [query.data, timezone])

  const earliest = firstAvailableDay(byDay)
  const page =
    userPage ??
    (earliest ? Math.min(MAX_PAGE, Math.floor(diffDays(today, earliest) / DAYS_PER_PAGE)) : 0)

  const pageFirst = addDays(today, page * DAYS_PER_PAGE)
  const days = Array.from({ length: DAYS_PER_PAGE }, (_, i) => addDays(pageFirst, i))
  const pageLast = days[days.length - 1]

  const availableOnPage = days.filter((d) => byDay.has(d))
  const selectedDay =
    userDay && days.includes(userDay) && byDay.has(userDay) ? userDay : (availableOnPage[0] ?? null)
  const options = selectedDay ? (byDay.get(selectedDay) ?? []) : []

  const nextAvailableAfterPage = firstAvailableDay(byDay, addDays(pageLast, 1))
  const lastWindowPage = windowOf(MAX_PAGE) * PAGES_PER_WINDOW + PAGES_PER_WINDOW - 1

  const goToDay = (day: string) => {
    setUserPage(Math.min(MAX_PAGE, Math.floor(diffDays(today, day) / DAYS_PER_PAGE)))
    setUserDay(day)
  }

  const loading = query.isPending

  return (
    <section className="picker" aria-label="選擇日期與時間" aria-busy={loading}>
      <div className="strip-head">
        <button
          type="button"
          className="icon-btn"
          aria-label="上一週"
          disabled={page === 0}
          onClick={() => {
            setUserPage(page - 1)
            setUserDay(null)
          }}
        >
          <ChevronLeftIcon width={20} height={20} />
        </button>
        <h3 className="strip-title">{formatMonthSpan(days[0], pageLast)}</h3>
        <button
          type="button"
          className="icon-btn"
          aria-label="下一週"
          disabled={page >= Math.min(MAX_PAGE, lastWindowPage)}
          onClick={() => {
            setUserPage(page + 1)
            setUserDay(null)
          }}
        >
          <ChevronRightIcon width={20} height={20} />
        </button>
      </div>

      <div className="strip" role="group" aria-label="日期">
        {days.map((day) => {
          // 資料還沒回來時是「不知道」,不能當成「沒有空檔」(否則載入中整排日期會被劃掉)
          const known = !!query.data
          const available = byDay.has(day)
          const noSlots = known && !available
          const selected = day === selectedDay
          return (
            <button
              key={day}
              type="button"
              className={`day ${selected ? 'day-on' : ''} ${noSlots ? 'day-off' : ''} ${day === today ? 'day-today' : ''}`}
              aria-pressed={selected}
              aria-label={`${formatDayLong(day)}${noSlots ? ',沒有空檔' : ''}`}
              disabled={!available}
              onClick={() => setUserDay(day)}
            >
              <span className="day-wd">{weekdayShort(day)}</span>
              <span className="day-num">{dayOfMonth(day)}</span>
            </button>
          )
        })}
      </div>

      <p className="tz-note">時間以店家當地時間({tzLabel(timezone)})顯示</p>

      {query.isPending && <Loading label="查詢可預約時段…" />}
      {query.isError && !query.data && (
        <ErrorState error={query.error} onRetry={() => query.refetch()} />
      )}

      {query.data && selectedDay && (
        <div className="slots">
          <h4 className="slots-title">{formatDayLong(selectedDay)}</h4>
          {groupByPeriod(options, timezone).map((group) => (
            <div key={group.period} className="slot-group">
              <div className="slot-period">{PERIOD_LABEL[group.period]}</div>
              <div className="slot-grid">
                {group.options.map((option) => (
                  <button
                    key={option.start}
                    type="button"
                    className={`slot ${option.start === selectedStart ? 'slot-on' : ''}`}
                    aria-pressed={option.start === selectedStart}
                    onClick={() => onSelect(option)}
                  >
                    {formatTime(option.start, timezone)}
                  </button>
                ))}
              </div>
            </div>
          ))}
        </div>
      )}

      {query.data && !selectedDay && (
        <div className="empty">
          {nextAvailableAfterPage ? (
            <>
              <p>這一週沒有空檔。</p>
              <p className="muted">最近可預約的日期是 {formatDayLong(nextAvailableAfterPage)}。</p>
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => goToDay(nextAvailableAfterPage)}
              >
                前往最近可預約日
              </button>
            </>
          ) : page < Math.min(MAX_PAGE, lastWindowPage) ? (
            <>
              <p>這段期間沒有空檔。</p>
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => {
                  setUserPage(Math.min(MAX_PAGE, (windowOf(page) + 1) * PAGES_PER_WINDOW))
                  setUserDay(null)
                }}
              >
                查看後面的日期
              </button>
            </>
          ) : (
            <p>近期沒有空檔了,請稍後再回來看看,或直接聯絡店家。</p>
          )}
        </div>
      )}
    </section>
  )
}
