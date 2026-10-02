import { useSyncExternalStore } from 'react'
import type { User } from './types'

// 登入狀態:JWT 存在 localStorage(重新整理 / 開新分頁仍保持登入)。
// 取捨:localStorage 可被頁面內的 XSS 讀到,所以前端用嚴格的 CSP(只允許同源腳本)降低風險;
// 更徹底的作法是 httpOnly cookie,需要後端配合,尚未做。
const KEY = 'tenant-saas.session'

export interface Session {
  token: string
  /** JWT 到期時間(毫秒) */
  expiresAt: number
  user: User
}

/** 解出 JWT 的 exp(秒)。只用來決定何時該請使用者重新登入,驗證仍由後端負責 */
export function jwtExpiry(token: string): number | null {
  try {
    const payload = token.split('.')[1]
    const json = atob(payload.replace(/-/g, '+').replace(/_/g, '/'))
    const exp = (JSON.parse(json) as { exp?: unknown }).exp
    return typeof exp === 'number' ? exp * 1000 : null
  } catch {
    return null
  }
}

function read(): Session | null {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return null
    const s = JSON.parse(raw) as Session
    if (!s.token || typeof s.expiresAt !== 'number' || s.expiresAt <= Date.now()) {
      localStorage.removeItem(KEY)
      return null
    }
    return s
  } catch {
    return null // localStorage 被停用或內容損壞:視為未登入
  }
}

let current: Session | null = read()
let timer: ReturnType<typeof setTimeout> | undefined
const listeners = new Set<() => void>()

function emit() {
  listeners.forEach((l) => l())
}

function scheduleExpiry() {
  if (timer) clearTimeout(timer)
  if (!current) return
  // setTimeout 上限約 24.8 天;JWT 遠小於此
  timer = setTimeout(() => clearSession(), Math.max(0, current.expiresAt - Date.now()))
}
scheduleExpiry()

export function getSession(): Session | null {
  return current
}

export function setSession(token: string, user: User): Session | null {
  const expiresAt = jwtExpiry(token)
  if (!expiresAt) return null
  current = { token, expiresAt, user }
  try {
    localStorage.setItem(KEY, JSON.stringify(current))
  } catch {
    // 存不進去就只在這個分頁有效
  }
  scheduleExpiry()
  emit()
  return current
}

export function clearSession() {
  current = null
  if (timer) clearTimeout(timer)
  try {
    localStorage.removeItem(KEY)
  } catch {
    // 忽略
  }
  emit()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function useSession(): Session | null {
  return useSyncExternalStore(subscribe, getSession)
}

// 其他分頁登出 / 登入時同步
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key === KEY) {
      current = read()
      scheduleExpiry()
      emit()
    }
  })
}
