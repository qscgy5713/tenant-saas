import type { WorkingHour } from '../admin/types'

/** 一天裡的營業時段,"HH:MM",店家當地時間 */
export interface Range {
  start: string
  end: string
}

/** 0 = 週日 … 6 = 週六 → 當天的營業時段(沒有 = 公休) */
export type WeekState = Record<number, Range[]>

/** 畫面上由週一排到週日(後端的 weekday 0 是週日) */
export const WEEK_ORDER = [1, 2, 3, 4, 5, 6, 0] as const

export const WEEKDAY_LABEL: Record<number, string> = {
  0: '週日',
  1: '週一',
  2: '週二',
  3: '週三',
  4: '週四',
  5: '週五',
  6: '週六',
}

export const emptyWeek = (): WeekState => ({ 0: [], 1: [], 2: [], 3: [], 4: [], 5: [], 6: [] })

export function fromHours(hours: WorkingHour[]): WeekState {
  const week = emptyWeek()
  for (const h of hours) week[h.weekday]?.push({ start: h.start, end: h.end })
  for (const day of WEEK_ORDER) week[day].sort((a, b) => a.start.localeCompare(b.start))
  return week
}

export function toHours(week: WeekState): WorkingHour[] {
  return WEEK_ORDER.flatMap((weekday) =>
    [...week[weekday]]
      .sort((a, b) => a.start.localeCompare(b.start))
      .map((r) => ({ weekday, start: r.start, end: r.end })),
  )
}

/** 回傳「有問題的那一天 → 說明」;全部正確則是空物件。規則與後端一致:結束要晚於開始,同一天不可重疊(相鄰可以) */
export function validateWeek(week: WeekState): Record<number, string> {
  const errors: Record<number, string> = {}
  for (const day of WEEK_ORDER) {
    const ranges = week[day]
    if (ranges.some((r) => !/^\d{2}:\d{2}$/.test(r.start) || !/^\d{2}:\d{2}$/.test(r.end))) {
      errors[day] = '請填寫完整的開始與結束時間'
      continue
    }
    if (ranges.some((r) => r.end <= r.start)) {
      errors[day] = '結束時間必須晚於開始時間'
      continue
    }
    const sorted = [...ranges].sort((a, b) => a.start.localeCompare(b.start))
    if (sorted.some((r, i) => i > 0 && r.start < sorted[i - 1].end)) {
      errors[day] = '時段不可重疊'
    }
  }
  return errors
}

export function sameWeek(a: WeekState, b: WeekState): boolean {
  return JSON.stringify(toHours(a)) === JSON.stringify(toHours(b))
}

export const MAX_RANGES_TOTAL = 50

export function countRanges(week: WeekState): number {
  return WEEK_ORDER.reduce<number>((n, d) => n + week[d].length, 0)
}

/** 常用的快速設定 */
export function presetWeek(days: readonly number[], start = '09:00', end = '18:00'): WeekState {
  const week = emptyWeek()
  for (const d of days) week[d] = [{ start, end }]
  return week
}
