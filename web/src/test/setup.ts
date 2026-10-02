import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'
import { afterAll, afterEach, beforeAll, vi } from 'vitest'
import { server } from './server'

// 未宣告的請求一律失敗,避免測試悄悄打到不存在的網址
beforeAll(() => server.listen({ onUnhandledRequest: 'error' }))
afterEach(() => {
  cleanup()
  server.resetHandlers()
  vi.useRealTimers()
})
afterAll(() => server.close())

if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {}
}
