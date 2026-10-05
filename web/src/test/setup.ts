import '@testing-library/jest-dom/vitest'
import { cleanup, configure } from '@testing-library/react'
import { afterAll, afterEach, beforeAll, beforeEach, vi } from 'vitest'
import { clearSession } from '../admin/session'
import { server } from './server'

// findBy* / waitFor 預設只等 1 秒。機器忙的時候(本機同時開著別的東西、CI 共用主機)會偶爾逾時,
// 造成「這次過、下次不過」的假失敗。放寬上限不會讓成功的測試變慢(一找到就結束),只影響失敗時多等一下
configure({ asyncUtilTimeout: 4000 })

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
