import { HttpResponse, http } from 'msw'
import type { MyShop, Role, User } from '../admin/types'
import { setSession } from '../admin/session'
import { API, server } from './server'

export const USER: User = { id: 'u-owner', email: 'owner@demo.example.com', name: '林美玲' }

/** 只有 exp 有意義的 JWT(前端只解 exp 決定何時過期,簽章由後端驗證) */
export function fakeJwt(expiresInSec = 3600): string {
  const b64 = (o: object) =>
    btoa(JSON.stringify(o)).replace(/=+$/, '').replace(/\+/g, '-').replace(/\//g, '_')
  return `${b64({ alg: 'HS256' })}.${b64({ sub: 'u', exp: Math.floor(Date.now() / 1000) + expiresInSec })}.sig`
}

export function loginAs(user: User = USER, expiresInSec = 3600) {
  const token = fakeJwt(expiresInSec)
  setSession(token, user)
  return token
}

export const SHOP = (role: Role = 'owner'): MyShop => ({
  id: 't1',
  slug: 'demo-salon',
  name: '森林系髮廊',
  timezone: 'Asia/Taipei',
  role,
})

/** 註冊「我在這家店的身分」,測試各角色看到的畫面 */
export function mockShopMe(role: Role = 'owner') {
  server.use(http.get(`${API}/t/demo-salon/me`, () => HttpResponse.json(SHOP(role))))
}

/** 記錄請求:回傳會被呼叫的次數與內容 */
export function spy<T = unknown>() {
  const calls: { url: URL; body: T | null; headers: Headers }[] = []
  return {
    calls,
    record: async (request: Request) => {
      let body: T | null = null
      try {
        body = (await request.clone().json()) as T
      } catch {
        // 沒有 body
      }
      calls.push({ url: new URL(request.url), body, headers: request.headers })
    },
  }
}

/**
 * 後台是 React.lazy 動態載入的,第一個渲染後台的測試要付出整包程式的編譯成本;
 * 機器忙碌時會超過 findBy 預設的 1 秒而偶爾失敗。測試檔在 beforeAll 呼叫它,先把成本移出測試本體。
 */
export async function preloadAdmin() {
  await Promise.all([import('../admin/AdminApp'), import('../admin/InvitationEntry')])
}
