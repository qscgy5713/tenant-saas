import { describe, expect, it } from 'vitest'
import {
  addDays,
  diffDays,
  formatDateTime,
  formatMonthSpan,
  formatTime,
  periodOf,
  todayYmd,
  tzLabel,
  weekdayShort,
  ymdInTz,
} from './time'

const TAIPEI = 'Asia/Taipei'

describe('ymdInTz:日期要依店家時區,不是瀏覽器時區', () => {
  it('UTC 16:30 在台北已經是隔天', () => {
    expect(ymdInTz('2026-10-05T16:30:00Z', TAIPEI)).toBe('2026-10-06')
    expect(ymdInTz('2026-10-05T15:59:00Z', TAIPEI)).toBe('2026-10-05')
  })

  it('同一時間點在不同時區是不同的日子', () => {
    const t = '2026-10-05T03:00:00Z'
    expect(ymdInTz(t, 'Pacific/Auckland')).toBe('2026-10-05')
    expect(ymdInTz(t, 'America/Los_Angeles')).toBe('2026-10-04')
  })

  it('todayYmd 以指定時間為準', () => {
    expect(todayYmd(TAIPEI, new Date('2026-12-31T16:00:00Z'))).toBe('2027-01-01')
  })
})

describe('日曆日運算', () => {
  it('跨月、跨年、閏年', () => {
    expect(addDays('2026-10-31', 1)).toBe('2026-11-01')
    expect(addDays('2026-12-31', 1)).toBe('2027-01-01')
    expect(addDays('2028-02-28', 1)).toBe('2028-02-29')
    expect(addDays('2026-03-01', -1)).toBe('2026-02-28')
    expect(addDays('2026-10-05', 0)).toBe('2026-10-05')
  })

  it('夏令時間切換日不會多一天或少一天', () => {
    // 美國 2026-03-08 跳時、11-01 回撥
    expect(addDays('2026-03-07', 1)).toBe('2026-03-08')
    expect(addDays('2026-03-08', 1)).toBe('2026-03-09')
    expect(addDays('2026-10-31', 1)).toBe('2026-11-01')
    expect(addDays('2026-11-01', 1)).toBe('2026-11-02')
  })

  it('diffDays', () => {
    expect(diffDays('2026-10-05', '2026-10-19')).toBe(14)
    expect(diffDays('2026-12-30', '2027-01-02')).toBe(3)
    expect(diffDays('2026-10-05', '2026-10-05')).toBe(0)
    expect(diffDays('2026-10-06', '2026-10-05')).toBe(-1)
  })

  it('星期不受執行環境時區影響', () => {
    expect(weekdayShort('2026-10-05')).toBe('週一')
    expect(weekdayShort('2026-10-04')).toBe('週日')
  })
})

describe('顯示格式', () => {
  it('時間是 24 小時制、依店家時區', () => {
    expect(formatTime('2026-10-05T06:30:00Z', TAIPEI)).toBe('14:30')
    expect(formatTime('2026-10-05T16:05:00Z', TAIPEI)).toBe('00:05')
    expect(formatTime('2026-10-05T04:00:00Z', 'UTC')).toBe('04:00')
  })

  it('完整日期時間', () => {
    // 日期與星期之間是否有空白,取決於 ICU 版本(Node 與瀏覽器不同),所以容許空白差異
    expect(formatDateTime('2026-10-05T06:30:00Z', TAIPEI)).toMatch(/^2026年10月5日\s?週一\s14:30$/)
  })

  it('月份標題:同月、跨月、跨年', () => {
    expect(formatMonthSpan('2026-10-05', '2026-10-11')).toBe('2026年10月')
    expect(formatMonthSpan('2026-10-28', '2026-11-03')).toBe('2026年10月 – 11月')
    expect(formatMonthSpan('2026-12-28', '2027-01-03')).toBe('2026年12月 – 2027年1月')
  })

  it('時區標籤隨夏令時間變動', () => {
    expect(tzLabel(TAIPEI)).toMatch(/\+8/)
    expect(tzLabel('America/New_York', '2026-01-15T12:00:00Z')).toMatch(/-5/)
    expect(tzLabel('America/New_York', '2026-07-15T12:00:00Z')).toMatch(/-4/)
  })
})

describe('periodOf:上午 / 下午 / 晚上', () => {
  it('邊界', () => {
    const at = (h: number) => `2026-10-05T${String(h - 8 + 24).padStart(2, '0')}:00:00Z`
    // 台北 = UTC+8:台北 11:59 → 03:59Z
    expect(periodOf('2026-10-05T03:59:00Z', TAIPEI)).toBe('morning')
    expect(periodOf('2026-10-05T04:00:00Z', TAIPEI)).toBe('afternoon')
    expect(periodOf('2026-10-05T09:59:00Z', TAIPEI)).toBe('afternoon')
    expect(periodOf('2026-10-05T10:00:00Z', TAIPEI)).toBe('evening')
    // 台北 00:00(前一天 16:00Z)
    expect(periodOf('2026-10-04T16:00:00Z', TAIPEI)).toBe('morning')
    expect(at(0)).toBeDefined()
  })
})

import { dayRange, startOfDay, toLocalInputs, weekdayIndex, zonedToUtc } from './time'

describe('zonedToUtc / startOfDay:店家當地時間 → UTC', () => {
  it('台北(UTC+8,無夏令時間)', () => {
    expect(zonedToUtc('2026-10-05', '10:30', TAIPEI).toISOString()).toBe('2026-10-05T02:30:00.000Z')
    expect(startOfDay('2026-10-05', TAIPEI).toISOString()).toBe('2026-10-04T16:00:00.000Z')
  })

  it('跨日:台北 00:00 在 UTC 是前一天', () => {
    expect(zonedToUtc('2026-01-01', '00:00', TAIPEI).toISOString()).toBe('2025-12-31T16:00:00.000Z')
  })

  it('偏移大的時區都正確(奧克蘭 +13、洛杉磯 -7、加拉巴哥群島 -6)', () => {
    expect(zonedToUtc('2026-10-05', '12:00', 'Pacific/Auckland').toISOString()).toBe(
      '2026-10-04T23:00:00.000Z',
    )
    expect(zonedToUtc('2026-10-05', '12:00', 'America/Los_Angeles').toISOString()).toBe(
      '2026-10-05T19:00:00.000Z',
    )
    expect(zonedToUtc('2026-10-05', '23:59', 'Pacific/Kiritimati').toISOString()).toBe(
      '2026-10-05T09:59:00.000Z',
    ) // UTC+14
  })

  it('夏令時間跳時:不存在的 02:30 → 跳完之後的第一個瞬間(03:00 EDT = 07:00Z)', () => {
    expect(zonedToUtc('2026-03-08', '02:30', 'America/New_York').toISOString()).toBe(
      '2026-03-08T07:00:00.000Z',
    )
    expect(zonedToUtc('2026-03-08', '01:59', 'America/New_York').toISOString()).toBe(
      '2026-03-08T06:59:00.000Z',
    )
  })

  it('夏令時間回撥:重複的 01:30 → 較早的那一個(EDT = 05:30Z),與後端規則一致', () => {
    expect(zonedToUtc('2026-11-01', '01:30', 'America/New_York').toISOString()).toBe(
      '2026-11-01T05:30:00.000Z',
    )
    expect(zonedToUtc('2026-11-01', '02:00', 'America/New_York').toISOString()).toBe(
      '2026-11-01T07:00:00.000Z',
    )
  })

  it('與 ymdInTz / formatTime 互為反函數', () => {
    for (const tz of [
      TAIPEI,
      'America/New_York',
      'Europe/London',
      'Pacific/Auckland',
      'Asia/Kolkata',
    ]) {
      const t = zonedToUtc('2026-06-15', '09:45', tz).toISOString()
      expect(ymdInTz(t, tz)).toBe('2026-06-15')
      expect(formatTime(t, tz)).toBe('09:45')
    }
  })
})

describe('dayRange', () => {
  it('一般的一天 24 小時', () => {
    const { from, to } = dayRange('2026-10-05', TAIPEI)
    expect(from).toBe('2026-10-04T16:00:00.000Z')
    expect(Date.parse(to) - Date.parse(from)).toBe(24 * 3_600_000)
  })

  it('夏令時間當天只有 23 小時(跳時)/ 25 小時(回撥)', () => {
    const spring = dayRange('2026-03-08', 'America/New_York')
    expect((Date.parse(spring.to) - Date.parse(spring.from)) / 3_600_000).toBe(23)
    const fall = dayRange('2026-11-01', 'America/New_York')
    expect((Date.parse(fall.to) - Date.parse(fall.from)) / 3_600_000).toBe(25)
  })

  it('相鄰兩天的區間剛好銜接,不重疊也不遺漏', () => {
    expect(dayRange('2026-10-05', TAIPEI).to).toBe(dayRange('2026-10-06', TAIPEI).from)
  })
})

describe('toLocalInputs / weekdayIndex', () => {
  it('UTC 瞬間 → 店家當地的日期與時間', () => {
    expect(toLocalInputs('2026-10-05T16:30:00Z', TAIPEI)).toEqual({
      ymd: '2026-10-06',
      hhmm: '00:30',
    })
  })

  it('星期:0 = 週日', () => {
    expect(weekdayIndex('2026-10-04')).toBe(0)
    expect(weekdayIndex('2026-10-05')).toBe(1)
    expect(weekdayIndex('2026-10-10')).toBe(6)
  })
})
