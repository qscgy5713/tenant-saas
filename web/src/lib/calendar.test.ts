import { describe, expect, it } from 'vitest'
import {
  addMonths,
  groupByLocalDay,
  hourWindow,
  layoutDay,
  startAtUnit,
  unitFromTop,
  minutesWithin,
  monthStart,
  monthWeeks,
  weekDays,
} from './calendar'

const TZ = 'Asia/Taipei'
const span = (day: string, from: string, to: string, id = `${day}${from}`) => ({
  id,
  starts_at: `${day}T${from}:00+08:00`,
  ends_at: `${day}T${to}:00+08:00`,
})

describe('weekDays', () => {
  it('週一開始,涵蓋包含該日的七天(週日屬於前一週)', () => {
    expect(weekDays('2026-10-05')[0]).toBe('2026-10-05') // 週一
    expect(weekDays('2026-10-11')).toEqual([
      '2026-10-05',
      '2026-10-06',
      '2026-10-07',
      '2026-10-08',
      '2026-10-09',
      '2026-10-10',
      '2026-10-11',
    ])
    expect(weekDays('2026-10-07')[0]).toBe('2026-10-05')
  })

  it('跨月、跨年也正確', () => {
    expect(weekDays('2026-12-31')).toContain('2027-01-03')
    expect(weekDays('2026-03-01')[0]).toBe('2026-02-23')
  })
})

describe('minutesWithin', () => {
  it('以店家時區計算,而不是 UTC', () => {
    // 2026-10-05 00:30 +08:00 = 10-04 16:30 UTC,店家當地仍是 10-05
    expect(minutesWithin(span('2026-10-05', '00:30', '01:30'), '2026-10-05', TZ)).toEqual([30, 90])
    expect(minutesWithin(span('2026-10-05', '00:30', '01:30'), '2026-10-04', TZ)).toBeNull()
  })

  it('跨午夜:兩天各截到當天邊界;結束剛好在午夜不會溢到隔天', () => {
    const late = span('2026-10-05', '23:00', '23:00')
    late.ends_at = '2026-10-06T01:00:00+08:00'
    expect(minutesWithin(late, '2026-10-05', TZ)).toEqual([23 * 60, 24 * 60])
    expect(minutesWithin(late, '2026-10-06', TZ)).toEqual([0, 60])
    const toMidnight = span('2026-10-05', '22:00', '22:00')
    toMidnight.ends_at = '2026-10-06T00:00:00+08:00'
    expect(minutesWithin(toMidnight, '2026-10-05', TZ)).toEqual([22 * 60, 24 * 60])
    expect(minutesWithin(toMidnight, '2026-10-06', TZ)).toBeNull()
  })

  it('夏令時間:位置以當地牆上時間為準', () => {
    // 紐約 2026-03-08 有夏令時間切換;下午 3 點仍然顯示在 15:00
    const s = { starts_at: '2026-03-08T19:00:00Z', ends_at: '2026-03-08T20:00:00Z' } // 15:00–16:00 EDT
    expect(minutesWithin(s, '2026-03-08', 'America/New_York')).toEqual([15 * 60, 16 * 60])
  })
})

describe('hourWindow', () => {
  it('預設 9–19;有預約超出就撐開(取整點)', () => {
    expect(hourWindow([], ['2026-10-05'], TZ)).toEqual([9, 19])
    expect(hourWindow([span('2026-10-05', '10:00', '11:00')], ['2026-10-05'], TZ)).toEqual([9, 19])
    expect(
      hourWindow(
        [span('2026-10-05', '07:30', '08:15'), span('2026-10-05', '20:00', '21:30')],
        ['2026-10-05'],
        TZ,
      ),
    ).toEqual([7, 22])
  })

  it('只看顯示範圍內的日子', () => {
    expect(hourWindow([span('2026-10-20', '06:00', '07:00')], ['2026-10-05'], TZ)).toEqual([9, 19])
  })
})

describe('layoutDay', () => {
  const d = '2026-10-05'

  it('位置與長度以 15 分鐘為一格,相對於視窗起點', () => {
    const [b] = layoutDay([span(d, '10:30', '11:30')], d, TZ, 9)
    expect(b).toMatchObject({ top: 6, len: 4, lane: 0, lanes: 1 })
  })

  it('太短的預約至少 2 格(否則點不到)', () => {
    const [b] = layoutDay([span(d, '10:00', '10:15')], d, TZ, 9)
    expect(b.len).toBe(2)
  })

  it('重疊的並排成多欄;不重疊的各自獨佔整欄', () => {
    const blocks = layoutDay(
      [
        span(d, '10:00', '11:00', 'a'),
        span(d, '10:30', '11:30', 'b'),
        span(d, '14:00', '15:00', 'c'),
      ],
      d,
      TZ,
      9,
    )
    const by = Object.fromEntries(blocks.map((x) => [x.item.id, x]))
    expect([by.a.lane, by.a.lanes]).toEqual([0, 2])
    expect([by.b.lane, by.b.lanes]).toEqual([1, 2])
    expect([by.c.lane, by.c.lanes]).toEqual([0, 1])
  })

  it('欄位會重複利用:a 結束後,c 可以接在同一欄,整群仍是 2 欄', () => {
    const blocks = layoutDay(
      [
        span(d, '10:00', '11:00', 'a'),
        span(d, '10:00', '12:00', 'b'),
        span(d, '11:00', '12:00', 'c'),
      ],
      d,
      TZ,
      9,
    )
    const by = Object.fromEntries(blocks.map((x) => [x.item.id, x]))
    expect(by.c.lane).toBe(by.a.lane)
    expect(blocks.every((x) => x.lanes === 2)).toBe(true)
  })

  it('首尾相接(10–11 與 11–12)不算重疊', () => {
    const blocks = layoutDay(
      [span(d, '10:00', '11:00', 'a'), span(d, '11:00', '12:00', 'b')],
      d,
      TZ,
      9,
    )
    expect(blocks.every((x) => x.lanes === 1 && x.lane === 0)).toBe(true)
  })

  it('超過上限的欄數會被壓在最後一欄,不會產生不存在的 class', () => {
    const many = Array.from({ length: 9 }, (_, i) => span(d, '10:00', '11:00', `x${i}`))
    const blocks = layoutDay(many, d, TZ, 9)
    expect(Math.max(...blocks.map((x) => x.lane))).toBe(5)
    expect(blocks.every((x) => x.lanes === 6)).toBe(true)
  })

  it('只包含屬於這一天的', () => {
    expect(layoutDay([span('2026-10-06', '10:00', '11:00')], d, TZ, 9)).toEqual([])
  })
})

describe('addMonths / monthStart', () => {
  it('回傳目標月份的第一天,跨年、31 號都正確', () => {
    expect(monthStart('2026-10-17')).toBe('2026-10-01')
    expect(addMonths('2026-01-31', 1)).toBe('2026-02-01')
    expect(addMonths('2026-12-15', 1)).toBe('2027-01-01')
    expect(addMonths('2026-01-15', -1)).toBe('2025-12-01')
    expect(addMonths('2026-10-01', -13)).toBe('2025-09-01')
    expect(addMonths('2026-10-01', 0)).toBe('2026-10-01')
  })
})

describe('monthWeeks', () => {
  it('週一開始、每週七天、涵蓋整個月', () => {
    const weeks = monthWeeks('2026-10-17') // 10/1 是週四、10/31 是週六
    expect(weeks[0][0]).toBe('2026-09-28')
    expect(weeks[0][3]).toBe('2026-10-01')
    expect(weeks.at(-1)![6]).toBe('2026-11-01')
    expect(weeks).toHaveLength(5)
    expect(weeks.every((w) => w.length === 7)).toBe(true)
    // 連續、沒有缺日
    const flat = weeks.flat()
    expect(flat.slice(1).every((d, i) => d > flat[i])).toBe(true)
  })

  it('剛好 4 週(2026 年 2 月從週日以外的週一開始…)與 6 週的情況', () => {
    expect(monthWeeks('2027-02-10')).toHaveLength(4) // 2027/2/1 是週一、共 28 天
    expect(monthWeeks('2026-08-10')).toHaveLength(6) // 8/1 是週六、8/31 是週一
  })
})

describe('groupByLocalDay', () => {
  it('依店家時區分組:台北凌晨 00:30 是當天,不是前一天 UTC', () => {
    const items = [
      span('2026-10-05', '23:30', '23:45', 'late'),
      span('2026-10-05', '00:30', '01:00', 'early'),
      span('2026-10-06', '09:00', '10:00', 'next'),
    ]
    // 真實 API 回的是 UTC(Z 結尾):台北 10/5 00:30 其實是 UTC 10/4 16:30
    const utc = items.map((i) => ({
      ...i,
      starts_at: new Date(i.starts_at).toISOString(),
      ends_at: new Date(i.ends_at).toISOString(),
    }))
    expect(utc[1].starts_at.startsWith('2026-10-04')).toBe(true)
    const map = groupByLocalDay(utc, TZ)
    expect([...map.keys()].sort()).toEqual(['2026-10-05', '2026-10-06'])
    expect(map.get('2026-10-05')!.map((x) => x.id)).toEqual(['early', 'late'])
  })
})

describe('unitFromTop / startAtUnit:拖曳放開的位置 → 時間', () => {
  // 欄高 480px、40 格(10 小時 × 4)→ 每格 12px
  it('吸附到最近的一格;夾在 [0, 最後一格]', () => {
    expect(unitFromTop(0, 480, 40)).toBe(0)
    expect(unitFromTop(5, 480, 40)).toBe(0) // 不到半格 → 往下一格以前的那格
    expect(unitFromTop(7, 480, 40)).toBe(1) // 過半格 → 進位
    expect(unitFromTop(240, 480, 40)).toBe(20)
    expect(unitFromTop(-300, 480, 40)).toBe(0)
    expect(unitFromTop(9999, 480, 40)).toBe(39)
  })

  it('欄高或格數為 0(尚未排版)不會算出 NaN', () => {
    expect(unitFromTop(100, 0, 40)).toBe(0)
    expect(unitFromTop(100, 480, 0)).toBe(0)
  })

  it('格數 → 店家當地時間的 UTC 瞬間', () => {
    // 從 9 點起算第 28 格 = 16:00(台北,UTC+8)
    expect(startAtUnit('2026-10-05', 9, 28, 'Asia/Taipei')).toBe('2026-10-05T08:00:00.000Z')
    expect(startAtUnit('2026-10-05', 9, 0, 'Asia/Taipei')).toBe('2026-10-05T01:00:00.000Z')
    expect(startAtUnit('2026-10-05', 9, 1, 'Asia/Taipei')).toBe('2026-10-05T01:15:00.000Z')
    // 視窗被撐到凌晨時,最後一格仍在同一天
    expect(startAtUnit('2026-10-05', 0, 95, 'Asia/Taipei')).toBe('2026-10-05T15:45:00.000Z')
  })

  it('用的是店家時區,不是瀏覽器時區', () => {
    expect(startAtUnit('2026-10-05', 9, 0, 'America/New_York')).toBe('2026-10-05T13:00:00.000Z')
  })
})
