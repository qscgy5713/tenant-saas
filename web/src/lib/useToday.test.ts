import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { msUntilNextDay, useToday } from './useToday'

// 2026-10-05 23:50 台北 = 15:50 UTC
const BEFORE_MIDNIGHT = new Date('2026-10-05T15:50:00Z')

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date', 'setTimeout', 'clearTimeout'] })
  vi.setSystemTime(BEFORE_MIDNIGHT)
})
afterEach(() => vi.useRealTimers())

describe('msUntilNextDay', () => {
  it('算到店家時區的下一個午夜(不是 UTC 的午夜),多留 1 秒', () => {
    // 台北 23:50 → 10 分鐘 + 1 秒
    expect(msUntilNextDay('Asia/Taipei', BEFORE_MIDNIGHT)).toBe(10 * 60_000 + 1000)
    // 同一個瞬間在紐約(UTC-4)是 11:50,距離午夜 12 小時 10 分鐘
    expect(msUntilNextDay('America/New_York', BEFORE_MIDNIGHT)).toBe(
      12 * 3_600_000 + 10 * 60_000 + 1000,
    )
  })
  it('剛好午夜之後不會是 0(至少 1 秒,避免計時器空轉)', () => {
    expect(msUntilNextDay('Asia/Taipei', new Date('2026-10-05T16:00:00Z'))).toBeGreaterThanOrEqual(
      1000,
    )
  })
})

describe('useToday', () => {
  it('跨過店家時區的午夜 → 自動換成新的一天', () => {
    const { result } = renderHook(() => useToday('Asia/Taipei'))
    expect(result.current.today).toBe('2026-10-05')
    act(() => {
      vi.advanceTimersByTime(10 * 60_000 + 1000)
    })
    expect(result.current.today).toBe('2026-10-06')
    expect(result.current.now.toISOString()).toBe('2026-10-05T16:00:01.000Z')
  })

  it('換日後會再排下一個午夜,所以可以連續跨很多天', () => {
    const { result } = renderHook(() => useToday('Asia/Taipei'))
    act(() => {
      vi.advanceTimersByTime(10 * 60_000 + 1000)
    })
    expect(result.current.today).toBe('2026-10-06')
    act(() => {
      vi.advanceTimersByTime(24 * 3_600_000)
    })
    expect(result.current.today).toBe('2026-10-07')
  })

  it('同一天內 `now` 保持同一個物件(拿它當查詢條件不會一直重新請求)', () => {
    const { result, rerender } = renderHook(() => useToday('Asia/Taipei'))
    const first = result.current.now
    rerender()
    act(() => {
      vi.advanceTimersByTime(5 * 60_000) // 還沒到午夜
    })
    expect(result.current.now).toBe(first)
  })

  it('計時器沒觸發(背景分頁被節流 / 睡眠醒來),但分頁重新可見時會補上', () => {
    const { result } = renderHook(() => useToday('Asia/Taipei'))
    // 時間直接跳到隔天,計時器沒跑
    vi.setSystemTime(new Date('2026-10-06T02:00:00Z'))
    expect(result.current.today).toBe('2026-10-05')
    act(() => {
      document.dispatchEvent(new Event('visibilitychange'))
    })
    expect(result.current.today).toBe('2026-10-06')
  })

  it('視窗取得焦點時也會檢查;還是同一天就什麼都不變', () => {
    const { result } = renderHook(() => useToday('Asia/Taipei'))
    const first = result.current.now
    act(() => {
      window.dispatchEvent(new Event('focus'))
    })
    expect(result.current.now).toBe(first)

    vi.setSystemTime(new Date('2026-10-07T02:00:00Z'))
    act(() => {
      window.dispatchEvent(new Event('focus'))
    })
    expect(result.current.today).toBe('2026-10-07')
  })

  it('分頁被隱藏時(visibilityState 不是 visible)不更新', () => {
    const { result } = renderHook(() => useToday('Asia/Taipei'))
    vi.setSystemTime(new Date('2026-10-06T02:00:00Z'))
    const spy = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    act(() => {
      document.dispatchEvent(new Event('visibilitychange'))
    })
    expect(result.current.today).toBe('2026-10-05')
    spy.mockRestore()
  })

  it('用的是店家時區:同一個瞬間,不同時區的「今天」不同', () => {
    vi.setSystemTime(new Date('2026-10-05T20:00:00Z')) // 台北已是 10/6 04:00,紐約還是 10/5 16:00
    expect(renderHook(() => useToday('Asia/Taipei')).result.current.today).toBe('2026-10-06')
    expect(renderHook(() => useToday('America/New_York')).result.current.today).toBe('2026-10-05')
  })

  it('卸載後清掉計時器與事件監聽(不會對已卸載的元件更新)', () => {
    const { unmount } = renderHook(() => useToday('Asia/Taipei'))
    expect(vi.getTimerCount()).toBe(1)
    unmount()
    expect(vi.getTimerCount()).toBe(0)
    const remove = vi.spyOn(document, 'removeEventListener')
    const { unmount: u2 } = renderHook(() => useToday('Asia/Taipei'))
    u2()
    expect(remove).toHaveBeenCalledWith('visibilitychange', expect.any(Function))
  })
})
