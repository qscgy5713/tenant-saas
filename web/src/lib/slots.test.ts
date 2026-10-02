import { describe, expect, it } from 'vitest'
import type { Slot } from '../api/types'
import { firstAvailableDay, groupByDay, groupByPeriod } from './slots'

const TAIPEI = 'Asia/Taipei'
const slot = (start: string, staff: string): Slot => ({ staff_id: staff, start, end: start })

describe('groupByDay', () => {
  it('依店家當地日期分組(UTC 跨日的時段屬於店家當地的隔天)', () => {
    const grouped = groupByDay(
      [
        slot('2026-10-05T01:00:00Z', 'a'), // 台北 10/5 09:00
        slot('2026-10-05T16:30:00Z', 'a'), // 台北 10/6 00:30
      ],
      TAIPEI,
    )
    expect([...grouped.keys()]).toEqual(['2026-10-05', '2026-10-06'])
  })

  it('同一時間多位員工有空 → 合併成一個選項並保留所有員工', () => {
    const grouped = groupByDay(
      [slot('2026-10-05T02:00:00Z', 'b'), slot('2026-10-05T02:00:00Z', 'a')],
      TAIPEI,
    )
    const options = grouped.get('2026-10-05')!
    expect(options).toHaveLength(1)
    expect(options[0].staffIds.sort()).toEqual(['a', 'b'])
  })

  it('日期與時間都排序,不論輸入順序', () => {
    const grouped = groupByDay(
      [
        slot('2026-10-06T02:00:00Z', 'a'),
        slot('2026-10-05T03:00:00Z', 'a'),
        slot('2026-10-05T01:00:00Z', 'a'),
      ],
      TAIPEI,
    )
    expect([...grouped.keys()]).toEqual(['2026-10-05', '2026-10-06'])
    expect(grouped.get('2026-10-05')!.map((o) => o.start)).toEqual([
      '2026-10-05T01:00:00Z',
      '2026-10-05T03:00:00Z',
    ])
  })

  it('沒有時段 → 空', () => {
    expect(groupByDay([], TAIPEI).size).toBe(0)
  })
})

describe('firstAvailableDay', () => {
  const grouped = groupByDay(
    [slot('2026-10-05T01:00:00Z', 'a'), slot('2026-10-09T01:00:00Z', 'a')],
    TAIPEI,
  )
  it('第一個有空的日子', () => expect(firstAvailableDay(grouped)).toBe('2026-10-05'))
  it('可指定不早於某天', () => {
    expect(firstAvailableDay(grouped, '2026-10-06')).toBe('2026-10-09')
    expect(firstAvailableDay(grouped, '2026-10-10')).toBeNull()
  })
  it('沒資料 → null', () => expect(firstAvailableDay(new Map())).toBeNull())
})

describe('groupByPeriod', () => {
  it('依上午 / 下午 / 晚上分組,缺的時段不出現', () => {
    const options = [
      { start: '2026-10-05T01:00:00Z', staffIds: ['a'] }, // 09:00
      { start: '2026-10-05T05:00:00Z', staffIds: ['a'] }, // 13:00
      { start: '2026-10-05T11:00:00Z', staffIds: ['a'] }, // 19:00
      { start: '2026-10-05T02:00:00Z', staffIds: ['a'] }, // 10:00
    ]
    const groups = groupByPeriod(options, TAIPEI)
    expect(groups.map((g) => g.period)).toEqual(['morning', 'afternoon', 'evening'])
    expect(groups[0].options).toHaveLength(2)
    expect(groupByPeriod([options[1]], TAIPEI).map((g) => g.period)).toEqual(['afternoon'])
  })
})
