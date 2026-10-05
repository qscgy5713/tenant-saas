import { useSyncExternalStore } from 'react'
import { getMe } from './api'
import type { User } from './types'

// 登入狀態:JWT 在後端發的 HttpOnly cookie 裡,前端的 JavaScript(包含被 XSS 注入的)讀不到,
// 所以這裡**不存 token**,只記「目前登入的是誰」。
// 重新整理後靠 GET /auth/me 向後端確認(cookie 會自動帶上);確認完成前是 loading,
// 路由守衛要等它,否則重新整理會閃一下登入頁。

export interface Session {
  user: User
}

export type SessionStatus = 'loading' | 'in' | 'out'

/** 舊版把 JWT 存在 localStorage。升級後立刻丟掉,不要讓舊 token 留在瀏覽器裡 */
const LEGACY_KEY = 'tenant-saas.session'
/** 跨分頁同步用的訊號(沒有任何機密):其他分頁登入 / 登出時改寫它,這邊收到就重新向後端確認 */
const SYNC_KEY = 'tenant-saas.auth-event'

let status: SessionStatus = 'loading'
let current: Session | null = null
/** 每次登入狀態被明確改變就 +1;進行中的 restoreSession 發現過期就放棄結果,避免蓋掉剛登入的人 */
let epoch = 0
const listeners = new Set<() => void>()

try {
  localStorage.removeItem(LEGACY_KEY)
} catch {
  // localStorage 被停用:沒有舊資料可清
}

function emit() {
  listeners.forEach((l) => l())
}

function announce() {
  try {
    localStorage.setItem(SYNC_KEY, String(Date.now()))
  } catch {
    // 無法通知其他分頁:它們下一次 API 回 401 時仍會登出
  }
}

function apply(next: Session | null) {
  current = next
  status = next ? 'in' : 'out'
  emit()
}

export function getSession(): Session | null {
  return current
}

export function getStatus(): SessionStatus {
  return status
}

export function setSession(user: User) {
  epoch++
  apply({ user })
  announce()
}

/** 清掉前端的登入狀態。後端的 cookie 要靠 POST /auth/logout 清(JS 刪不掉 HttpOnly) */
export function clearSession() {
  epoch++
  const wasIn = current !== null
  apply(null)
  if (wasIn) announce()
}

/** 問後端「我現在登入了嗎」。任何失敗(401、連不上)都當作未登入 */
export async function restoreSession(): Promise<void> {
  const mine = epoch
  let user: User | null = null
  try {
    user = await getMe()
  } catch {
    user = null
  }
  if (mine !== epoch) return
  apply(user ? { user } : null)
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function useSession(): Session | null {
  return useSyncExternalStore(subscribe, getSession)
}

export function useSessionStatus(): SessionStatus {
  return useSyncExternalStore(subscribe, getStatus)
}

// 其他分頁登入 / 登出 → 重新向後端確認
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key === SYNC_KEY) void restoreSession()
  })
}
