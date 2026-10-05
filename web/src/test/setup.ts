import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'
import { afterAll, afterEach, beforeAll, beforeEach, vi } from 'vitest'
import { clearSession } from '../admin/session'
import { server } from './server'

// 未宣告的請求一律失敗,避免測試悄悄打到不存在的網址
beforeAll(() => server.listen({ onUnhandledRequest: 'error' }))
// 模組載入時登入狀態是 loading(等著問後端);測試一律從「確定未登入」開始
beforeEach(() => clearSession())
afterEach(() => {
  cleanup()
  server.resetHandlers()
  clearSession()
  localStorage.clear()
  vi.useRealTimers()
})
afterAll(() => server.close())

if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {}
}

// jsdom 沒有實作 <dialog> 的 showModal / close
if (typeof HTMLDialogElement !== 'undefined') {
  HTMLDialogElement.prototype.showModal ||= function showModal(this: HTMLDialogElement) {
    this.setAttribute('open', '')
  }
  HTMLDialogElement.prototype.close ||= function close(this: HTMLDialogElement) {
    this.removeAttribute('open')
    this.dispatchEvent(new Event('close'))
  }
}
