import { HttpResponse, http } from 'msw'
import type { MyShop, Role, User } from '../admin/types'
import { setSession } from '../admin/session'
import { API, server } from './server'

export const USER: User = { id: 'u-owner', email: 'owner@demo.example.com', name: '林美玲' }

/** 模擬「已登入」:登入靠 HttpOnly cookie,前端只記使用者是誰 */
export function loginAs(user: User = USER) {
  setSession(user)
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
  server.use(
    http.get(`${API}/t/demo-salon/me`, () => HttpResponse.json(SHOP(role))),
    // 預設未啟用線上付款;要測計費的測試再自己覆蓋
    http.get(`${API}/t/demo-salon/billing`, () =>
      HttpResponse.json({
        enabled: false,
        purchasable: [],
        subscription: null,
        can_checkout: false,
        can_manage: false,
      }),
    ),
  )
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
