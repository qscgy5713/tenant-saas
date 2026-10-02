import { addDays, toLocalInputs, ymdInTz } from './time'

/** 日曆一格 = 15 分鐘;一天 96 格。位置與長度都用「格數」表示,畫面用 class(cal-top-N / cal-len-N),不用行內 style(CSP) */
export const UNIT_MINUTES = 15
export const UNITS_PER_HOUR = 60 / UNIT_MINUTES
/** 同一時段並排最多幾欄(超過的與最後一欄疊在一起,仍可從列表檢視看到) */
export const MAX_LANES = 6
/** 預設顯示的營業時段;資料超出時自動撐開 */
const DEFAULT_START_HOUR = 9
const DEFAULT_END_HOUR = 19

export interface Span {
  starts_at: string
  ends_at: string
}

export interface PlacedBlock<T extends Span> {
  item: T
  /** 距離視窗頂端幾格 */
  top: number
  /** 佔幾格(至少 2 格,否則點不到) */
  len: number
  lane: number
  lanes: number
}

const minutesOf = (hhmm: string) => {
  const [h, m] = hhmm.split(':').map(Number)
  return h * 60 + m
}

/** 預約在店家「某一天」內佔的分鐘範圍;跨午夜的部分截到當天 0:00 / 24:00。不在這天則回 null */
export function minutesWithin(span: Span, ymd: string, tz: string): [number, number] | null {
  const startDay = ymdInTz(span.starts_at, tz)
  // 結束時間剛好是午夜時,結束的「那一天」要算前一毫秒
  const endDay = ymdInTz(new Date(new Date(span.ends_at).getTime() - 1), tz)
  if (ymd < startDay || ymd > endDay) return null
  const start = ymd === startDay ? minutesOf(toLocalInputs(span.starts_at, tz).hhmm) : 0
  const end = ymd === endDay ? minutesOf(toLocalInputs(span.ends_at, tz).hhmm) : 24 * 60
  return [start, end === 0 ? 24 * 60 : end]
}

/** 要顯示的小時範圍 [startHour, endHour):預設 9–19,有預約超出就撐開(取整點) */
export function hourWindow<T extends Span>(
  items: T[],
  days: string[],
  tz: string,
): [number, number] {
  let lo = DEFAULT_START_HOUR
  let hi = DEFAULT_END_HOUR
  for (const item of items) {
    for (const day of days) {
      const range = minutesWithin(item, day, tz)
      if (!range) continue
      lo = Math.min(lo, Math.floor(range[0] / 60))
      hi = Math.max(hi, Math.ceil(range[1] / 60))
    }
  }
  return [lo, hi]
}

/**
 * 排出某一天的區塊:時間重疊的預約並排成多欄。
 * 重疊的一群(cluster)共用同一個欄數,每筆放進第一個不衝突的欄。
 */
export function layoutDay<T extends Span>(
  items: T[],
  ymd: string,
  tz: string,
  startHour: number,
): PlacedBlock<T>[] {
  const entries = items
    .map((item) => ({ item, range: minutesWithin(item, ymd, tz) }))
    .filter((e): e is { item: T; range: [number, number] } => e.range !== null)
    .sort((a, b) => a.range[0] - b.range[0] || b.range[1] - a.range[1])

  const placed: PlacedBlock<T>[] = []
  let cluster: { block: PlacedBlock<T>; end: number }[] = []
  let laneEnds: number[] = []
  let clusterEnd = -1

  const flush = () => {
    const lanes = Math.min(MAX_LANES, Math.max(1, laneEnds.length))
    for (const { block } of cluster) block.lanes = lanes
    cluster = []
    laneEnds = []
  }

  for (const { item, range } of entries) {
    if (range[0] >= clusterEnd) {
      flush()
      clusterEnd = -1
    }
    let lane = laneEnds.findIndex((end) => end <= range[0])
    if (lane === -1) lane = laneEnds.length
    laneEnds[lane] = range[1]
    clusterEnd = Math.max(clusterEnd, range[1])

    const top = Math.max(0, Math.round((range[0] - startHour * 60) / UNIT_MINUTES))
    const len = Math.max(2, Math.round((range[1] - range[0]) / UNIT_MINUTES))
    const block: PlacedBlock<T> = { item, top, len, lane: Math.min(lane, MAX_LANES - 1), lanes: 1 }
    cluster.push({ block, end: range[1] })
    placed.push(block)
  }
  flush()
  return placed
}

/** 包含 ymd 的那一週(週一開始)的七天 */
export function weekDays(ymd: string): string[] {
  const dow = new Date(`${ymd}T12:00:00Z`).getUTCDay() // 0 = 週日
  const monday = addDays(ymd, -((dow + 6) % 7))
  return Array.from({ length: 7 }, (_, i) => addDays(monday, i))
}

/** 該日所在月份的第一天 */
export const monthStart = (ymd: string) => `${ymd.slice(0, 7)}-01`

/** 往前 / 往後 n 個月,回傳那個月的第一天(不會有「31 號 + 1 個月」的問題) */
export function addMonths(ymd: string, n: number): string {
  const [y, m] = ymd.split('-').map(Number)
  const index = y * 12 + (m - 1) + n
  const year = Math.floor(index / 12)
  const month = (index % 12) + 1
  return `${year}-${String(month).padStart(2, '0')}-01`
}

/** 月曆格:週一開始,包含前後月份補滿的日子。每週七天,4–6 週 */
export function monthWeeks(ymd: string): string[][] {
  const first = monthStart(ymd)
  const next = addMonths(first, 1)
  const weeks: string[][] = []
  let cursor = weekDays(first)[0]
  while (cursor < next) {
    weeks.push(Array.from({ length: 7 }, (_, i) => addDays(cursor, i)))
    cursor = addDays(cursor, 7)
  }
  return weeks
}

/** 依店家當地日期分組(依開始時間),每組內依時間排序 */
export function groupByLocalDay<T extends Span>(items: T[], tz: string): Map<string, T[]> {
  const map = new Map<string, T[]>()
  for (const item of items) {
    const day = ymdInTz(item.starts_at, tz)
    const list = map.get(day) ?? []
    list.push(item)
    map.set(day, list)
  }
  for (const list of map.values()) list.sort((a, b) => a.starts_at.localeCompare(b.starts_at))
  return map
}
