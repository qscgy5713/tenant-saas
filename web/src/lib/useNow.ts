import { useEffect, useState } from 'react'

/** 目前時間(毫秒),每隔一段時間更新。頁面開著不動時,「預約已開始」之類的判斷也會跟著改變 */
export function useNow(intervalMs = 30_000): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs)
    return () => clearInterval(id)
  }, [intervalMs])
  return now
}
