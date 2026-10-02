import { describe, expect, it } from 'vitest'
import {
  countRanges,
  emptyWeek,
  fromHours,
  presetWeek,
  sameWeek,
  toHours,
  validateWeek,
  WEEK_ORDER,
} from './hours'

describe('fromHours / toHours', () => {
  it('互為反函數,而且排序穩定(週一排最前、週日最後,同一天依開始時間)', () => {
    const hours = [
      { weekday: 0, start: '10:00', end: '12:00' },
      { weekday: 1, start: '13:00', end: '18:00' },
      { weekday: 1, start: '09:00', end: '12:00' },
    ]
    const week = fromHours(hours)
    expect(week[1]).toEqual([
      { start: '09:00', end: '12:00' },
      { start: '13:00', end: '18:00' },
    ])
    expect(toHours(week)).toEqual([
      { weekday: 1, start: '09:00', end: '12:00' },
      { weekday: 1, start: '13:00', end: '18:00' },
      { weekday: 0, start: '10:00', end: '12:00' },
    ])
  })

  it('沒有資料 → 七天都公休', () => {
    const week = fromHours([])
    expect(WEEK_ORDER.every((d) => week[d].length === 0)).toBe(true)
    expect(toHours(week)).toEqual([])
  })
})

describe('validateWeek(規則與後端一致)', () => {
  const week = (day: number, ranges: { start: string; end: string }[]) => ({
    ...emptyWeek(),
    [day]: ranges,
  })

  it('合法:空、單一時段、相鄰時段(12:00 結束接 12:00 開始)', () => {
    expect(validateWeek(emptyWeek())).toEqual({})
    expect(validateWeek(week(1, [{ start: '09:00', end: '18:00' }]))).toEqual({})
    expect(
      validateWeek(
        week(1, [
          { start: '09:00', end: '12:00' },
          { start: '12:00', end: '15:00' },
        ]),
      ),
    ).toEqual({})
  })

  it('結束不晚於開始', () => {
    expect(validateWeek(week(2, [{ start: '12:00', end: '09:00' }]))[2]).toContain('晚於')
    expect(validateWeek(week(2, [{ start: '09:00', end: '09:00' }]))[2]).toContain('晚於')
  })

  it('重疊,不論輸入順序', () => {
    const a = week(3, [
      { start: '09:00', end: '12:00' },
      { start: '11:00', end: '15:00' },
    ])
    const b = week(3, [
      { start: '11:00', end: '15:00' },
      { start: '09:00', end: '12:00' },
    ])
    expect(validateWeek(a)[3]).toContain('重疊')
    expect(validateWeek(b)[3]).toContain('重疊')
  })

  it('沒填完整', () => {
    expect(validateWeek(week(4, [{ start: '', end: '10:00' }]))[4]).toContain('完整')
  })

  it('只標出有問題的那一天', () => {
    const w = {
      ...week(1, [{ start: '10:00', end: '09:00' }]),
      2: [{ start: '09:00', end: '10:00' }],
    }
    expect(Object.keys(validateWeek(w))).toEqual(['1'])
  })
})

describe('sameWeek / presetWeek / countRanges', () => {
  it('時段順序不同但內容相同 → 視為相同(不該出現「有未儲存的變更」)', () => {
    const a = fromHours([
      { weekday: 1, start: '09:00', end: '12:00' },
      { weekday: 1, start: '13:00', end: '18:00' },
    ])
    const b = {
      ...emptyWeek(),
      1: [
        { start: '13:00', end: '18:00' },
        { start: '09:00', end: '12:00' },
      ],
    }
    expect(sameWeek(a, b)).toBe(true)
    expect(sameWeek(a, emptyWeek())).toBe(false)
  })

  it('preset', () => {
    const w = presetWeek([1, 2, 3, 4, 5])
    expect(countRanges(w)).toBe(5)
    expect(w[6]).toEqual([])
    expect(w[1]).toEqual([{ start: '09:00', end: '18:00' }])
  })
})
