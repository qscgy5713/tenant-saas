import type { Slot } from '../api/types'
import { type DayPeriod, periodOf, ymdInTz } from './time'

export interface TimeOption {
  start: string
  /** 這個時間點有空的員工(不指定員工時可能不只一位) */
  staffIds: string[]
}

/** 依店家當地日期分組;同一個開始時間若多位員工都有空,合併成一個選項 */
export function groupByDay(slots: Slot[], timeZone: string): Map<string, TimeOption[]> {
  const byDay = new Map<string, Map<string, TimeOption>>()
  for (const slot of slots) {
    const day = ymdInTz(slot.start, timeZone)
    const options = byDay.get(day) ?? new Map<string, TimeOption>()
    const existing = options.get(slot.start)
    if (existing) existing.staffIds.push(slot.staff_id)
    else options.set(slot.start, { start: slot.start, staffIds: [slot.staff_id] })
    byDay.set(day, options)
  }

  const result = new Map<string, TimeOption[]>()
  for (const day of [...byDay.keys()].sort()) {
    result.set(
      day,
      [...byDay.get(day)!.values()].sort((a, b) => a.start.localeCompare(b.start)),
    )
  }
  return result
}

/** 第一個有空檔的日期(YYYY-MM-DD 字串可直接比較大小) */
export function firstAvailableDay(
  byDay: Map<string, TimeOption[]>,
  notBefore?: string,
): string | null {
  for (const day of byDay.keys()) {
    if (notBefore && day < notBefore) continue
    return day
  }
  return null
}

export function groupByPeriod(
  options: TimeOption[],
  timeZone: string,
): { period: DayPeriod; options: TimeOption[] }[] {
  const order: DayPeriod[] = ['morning', 'afternoon', 'evening']
  const groups = new Map<DayPeriod, TimeOption[]>()
  for (const option of options) {
    const period = periodOf(option.start, timeZone)
    groups.set(period, [...(groups.get(period) ?? []), option])
  }
  return order.filter((p) => groups.has(p)).map((p) => ({ period: p, options: groups.get(p)! }))
}
