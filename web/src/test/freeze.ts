import { vi } from 'vitest'
import { NOW } from './server'

/** 只固定 Date,不動計時器(React Query 與 userEvent 需要真實的 setTimeout) */
export function freezeNow(now: Date = NOW) {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(now)
}
