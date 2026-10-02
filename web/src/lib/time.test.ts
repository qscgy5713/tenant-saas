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
