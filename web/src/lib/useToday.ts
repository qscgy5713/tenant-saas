import { useEffect, useState } from 'react'
import { addDays, startOfDay, todayYmd } from './time'

/** 距離店家時區下一個午夜還有幾毫秒(多留 1 秒,確保醒來時已經過了午夜) */
export function msUntilNextDay(timezone: string, now: Date): number {
  const next = startOfDay(addDays(todayYmd(timezone, now), 1), timezone)
  // setTimeout 的上限約 24.8 天;一天遠小於它
  return Math.max(1000, next.getTime() - now.getTime() + 1000)
}

/**
 * 店家時區的「今天」與「現在」。頁面一直開著(櫃檯的平板整夜不關)時,跨過午夜會自動更新,
 * 不會隔天早上還顯示昨天的行程。
 *
 * - `now` 只在換日時才換成新的 Date:拿它當查詢條件不會一直重新請求(與之前 `useState(() => new Date())` 一樣穩定)
 * - 兩種更新時機:到午夜的計時器;分頁重新可見 / 視窗取得焦點時再檢查一次
 *   (背景分頁的計時器會被瀏覽器節流,筆電睡眠醒來計時器也不準,所以不能只靠計時器)
 */
export function useToday(timezone: string): { today: string; now: Date } {
  const [now, setNow] = useState(() => new Date())
  const today = todayYmd(timezone, now)

  useEffect(() => {
    const refresh = () =>
      setNow((prev) => {
        const current = new Date()
        return todayYmd(timezone, current) === todayYmd(timezone, prev) ? prev : current
      })

    const timer = setTimeout(refresh, msUntilNextDay(timezone, new Date()))
    const onVisible = () => {
      if (document.visibilityState === 'visible') refresh()
    }
    document.addEventListener('visibilitychange', onVisible)
    window.addEventListener('focus', refresh)
    return () => {
      clearTimeout(timer)
      document.removeEventListener('visibilitychange', onVisible)
      window.removeEventListener('focus', refresh)
    }
    // 換日後 `today` 改變 → 重新排下一個午夜的計時器
  }, [timezone, today])

  // 時區變了(例如店家改了時區)但 `now` 沒變:today 由 now 與 timezone 推導,所以直接就是對的
  return { today, now }
}
