// 預約是「實體世界的某個時間」,一律以店家的時區顯示,不是使用者瀏覽器的時區。
// 日期用 'YYYY-MM-DD' 字串表示(店家當地的日曆日),避免 Date 在不同時區解讀出不同的日子。

const LOCALE = 'zh-TW'

/** 某個時間點在指定時區是哪一天,'YYYY-MM-DD' */
export function ymdInTz(instant: Date | string, timeZone: string): string {
  // en-CA 的日期格式剛好是 YYYY-MM-DD
  return new Intl.DateTimeFormat('en-CA', {
    timeZone,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).format(new Date(instant))
}

export function todayYmd(timeZone: string, now: Date = new Date()): string {
  return ymdInTz(now, timeZone)
}

function parseYmd(ymd: string): [number, number, number] {
  const [y, m, d] = ymd.split('-').map(Number)
  return [y, m, d]
}

/** 日曆日加減天數。以 UTC 正午計算,不受夏令時間影響 */
export function addDays(ymd: string, days: number): string {
  const [y, m, d] = parseYmd(ymd)
  const date = new Date(Date.UTC(y, m - 1, d + days, 12))
  return date.toISOString().slice(0, 10)
}

export function diffDays(a: string, b: string): number {
  const [ay, am, ad] = parseYmd(a)
  const [by, bm, bd] = parseYmd(b)
  return Math.round((Date.UTC(by, bm - 1, bd) - Date.UTC(ay, am - 1, ad)) / 86_400_000)
}

/** 日曆日本身(不含時區概念)用 UTC 格式化 */
function formatCalendarDay(ymd: string, options: Intl.DateTimeFormatOptions): string {
  const [y, m, d] = parseYmd(ymd)
  return new Intl.DateTimeFormat(LOCALE, { timeZone: 'UTC', ...options }).format(
    new Date(Date.UTC(y, m - 1, d, 12)),
  )
}

/** 週列上的星期,例如「週一」 */
export function weekdayShort(ymd: string): string {
  return formatCalendarDay(ymd, { weekday: 'short' })
}

export function dayOfMonth(ymd: string): number {
  return parseYmd(ymd)[2]
}

/** 「10月5日(週一)」 */
export function formatDayLong(ymd: string): string {
  return formatCalendarDay(ymd, { month: 'long', day: 'numeric', weekday: 'short' })
}

/** 週列上方的月份標題:同月「2026年10月」,跨月「2026年10月 – 11月」 */
export function formatMonthSpan(firstYmd: string, lastYmd: string): string {
  const [fy, fm] = parseYmd(firstYmd)
  const [ly, lm] = parseYmd(lastYmd)
  if (fy === ly && fm === lm) return `${fy}年${fm}月`
  if (fy === ly) return `${fy}年${fm}月 – ${lm}月`
  return `${fy}年${fm}月 – ${ly}年${lm}月`
}

/** 24 小時制「14:30」 */
export function formatTime(instant: string, timeZone: string): string {
  return new Intl.DateTimeFormat(LOCALE, {
    timeZone,
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(new Date(instant))
}

/** 「2026年10月5日(週一)14:30」 */
export function formatDateTime(instant: string, timeZone: string): string {
  const day = new Intl.DateTimeFormat(LOCALE, {
    timeZone,
    year: 'numeric',
    month: 'long',
    day: 'numeric',
    weekday: 'short',
  }).format(new Date(instant))
  return `${day} ${formatTime(instant, timeZone)}`
}

/** 「GMT+8」。用 shortOffset 才能格式一致('short' 在 zh-TW 下台北給 GMT+8、紐約卻給 EST)。夏令時間下會隨日期變動,所以要帶時間點 */
export function tzLabel(timeZone: string, at: Date | string = new Date()): string {
  const part = new Intl.DateTimeFormat(LOCALE, { timeZone, timeZoneName: 'shortOffset' })
    .formatToParts(new Date(at))
    .find((p) => p.type === 'timeZoneName')
  return part?.value ?? timeZone
}

export type DayPeriod = 'morning' | 'afternoon' | 'evening'

export const PERIOD_LABEL: Record<DayPeriod, string> = {
  morning: '上午',
  afternoon: '下午',
  evening: '晚上',
}

/** 依店家當地小時分:上午 <12、下午 12–17、晚上 ≥18 */
export function periodOf(instant: string, timeZone: string): DayPeriod {
  const hour = Number(
    new Intl.DateTimeFormat('en-US', { timeZone, hour: '2-digit', hourCycle: 'h23' })
      .formatToParts(new Date(instant))
      .find((p) => p.type === 'hour')?.value,
  )
  if (hour < 12) return 'morning'
  if (hour < 18) return 'afternoon'
  return 'evening'
}
