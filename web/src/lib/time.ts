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

// ---------- 店家當地時間 ↔ UTC ----------
// 後台要把「店家當地的 10:30」或「店家當地的某一天」換成 UTC 時間點,
// 用二分搜尋找出「當地時鐘第一次到達該時刻」的瞬間,自然處理夏令時間:
//  - 跳時造成不存在的時刻 → 取跳完之後的第一個瞬間
//  - 回撥造成重複的時刻 → 取較早的那一個(與後端 availability.rs 的規則一致)

const wallFormatters = new Map<string, Intl.DateTimeFormat>()

/** 某瞬間在指定時區的牆上時鐘,'YYYY-MM-DDTHH:MM'(字串可直接比較先後) */
function wallClock(ms: number, timeZone: string): string {
  let fmt = wallFormatters.get(timeZone)
  if (!fmt) {
    fmt = new Intl.DateTimeFormat('en-CA', {
      timeZone,
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      hourCycle: 'h23',
    })
    wallFormatters.set(timeZone, fmt)
  }
  const p = Object.fromEntries(fmt.formatToParts(new Date(ms)).map((x) => [x.type, x.value]))
  return `${p.year}-${p.month}-${p.day}T${p.hour}:${p.minute}`
}

/** 店家當地的 `ymd` `hhmm`(例如 '2026-10-05'、'10:30')是哪個 UTC 瞬間 */
export function zonedToUtc(ymd: string, hhmm: string, timeZone: string): Date {
  const [y, m, d] = parseYmd(ymd)
  const target = `${ymd}T${hhmm}`
  const base = Date.UTC(y, m - 1, d)
  // 所有時區的偏移都在 -12 … +14 小時之間,答案一定落在這個範圍內
  let lo = base - 15 * 3_600_000 // 此時當地時鐘一定早於 target
  let hi = base + 40 * 3_600_000 // 此時當地時鐘一定晚於 target(涵蓋當天的任何時刻)
  while (hi - lo > 1) {
    const mid = Math.floor((lo + hi) / 2)
    if (wallClock(mid, timeZone) >= target) hi = mid
    else lo = mid
  }
  return new Date(hi)
}

/** 店家當地某一天 00:00 的 UTC 瞬間 */
export function startOfDay(ymd: string, timeZone: string): Date {
  return zonedToUtc(ymd, '00:00', timeZone)
}

/** 店家當地某一天的 [開始, 結束) 區間(結束是隔天 00:00),ISO 字串 */
export function dayRange(ymd: string, timeZone: string): { from: string; to: string } {
  return {
    from: startOfDay(ymd, timeZone).toISOString(),
    to: startOfDay(addDays(ymd, 1), timeZone).toISOString(),
  }
}

/** 把 UTC 瞬間拆成店家當地的日期與時間,給 <input type="date"> / <input type="time"> 用 */
export function toLocalInputs(instant: string, timeZone: string): { ymd: string; hhmm: string } {
  const [ymd, hhmm] = wallClock(new Date(instant).getTime(), timeZone).split('T')
  return { ymd, hhmm }
}

/** 當地日期的星期,0 = 週日(與後端 working_hours.weekday 一致) */
export function weekdayIndex(ymd: string): number {
  const [y, m, d] = parseYmd(ymd)
  return new Date(Date.UTC(y, m - 1, d, 12)).getUTCDay()
}
