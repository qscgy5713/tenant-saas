import { api, apiDownload, json } from '../api/client'
import type { Availability, BookingStatus, NewBooking, PublicBooking } from '../api/types'
import type {
  AdminBooking,
  AdminService,
  AuditPage,
  AuthResponse,
  Billing,
  BookingQuery,
  Customer,
  CustomerDetail,
  Invitation,
  Member,
  MyShop,
  Page,
  PlanInfo,
  PlanUsage,
  Role,
  ServicePage,
  TimeOff,
  TimeOffPage,
  User,
  WorkingHour,
} from './types'

const enc = encodeURIComponent

/**
 * 登入靠後端發的 HttpOnly cookie,瀏覽器對同源請求自動帶上,這裡不碰 token。
 * 所以 API 必須與頁面同源(正式環境由 nginx 代理 /api,開發由 Vite 代理)。
 * 沒登入由後端回 401,全域處理會導向登入頁。
 */
const call = <T>(path: string, init?: RequestInit) => api<T>(path, init)
const send = (method: 'PATCH' | 'PUT' | 'POST', body: unknown): RequestInit => ({
  method,
  body: JSON.stringify(body),
})

function query(params: Record<string, string | number | undefined>) {
  const q = new URLSearchParams()
  for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== '') q.set(k, String(v))
  const s = q.toString()
  return s ? `?${s}` : ''
}

// ---------- 帳號 ----------
export const login = (email: string, password: string) =>
  api<AuthResponse>('/auth/login', json({ email, password }))

export const register = (name: string, email: string, password: string) =>
  api<AuthResponse>('/auth/register', json({ name, email, password }))

export const forgotPassword = (email: string) =>
  api<{ message: string }>('/auth/forgot-password', json({ email }))

export const resetPassword = (token: string, password: string) =>
  api<{ message: string }>('/auth/reset-password', json({ token, password }))

export const getMe = () => call<User>('/auth/me')

/** 清掉登入 cookie(前端的 JS 刪不掉 HttpOnly cookie) */
export const logoutRequest = () => api<void>('/auth/logout', { method: 'POST' })

// ---------- 店家 ----------
export const listShops = () => call<MyShop[]>('/tenants')

export const createShop = (body: { slug: string; name: string; timezone: string }) =>
  call<MyShop>('/tenants', send('POST', body))

/** 修改店名 / 時區(僅店主)。網址代稱不可改 */
export const updateShop = (slug: string, body: { name?: string; timezone?: string }) =>
  call<MyShop>(`/t/${enc(slug)}`, send('PATCH', body))

export const getShop = (slug: string) => call<MyShop>(`/t/${enc(slug)}/me`)

// ---------- 預約 ----------
export const listBookings = (slug: string, q: BookingQuery = {}) =>
  call<Page<AdminBooking>>(
    `/t/${enc(slug)}/bookings${query({
      from: q.from,
      to: q.to,
      status: q.status,
      staff_id: q.staffId,
      limit: q.limit,
      offset: q.offset,
    })}`,
  )

export const createBooking = (slug: string, body: NewBooking) =>
  call<{ id: string; staff_id: string; starts_at: string; ends_at: string; manage_token: string }>(
    `/t/${enc(slug)}/bookings`,
    send('POST', body),
  )

/** 員工端:這筆預約改期時可選的時段(排除自己) */
export const getBookingAvailability = (slug: string, id: string, from: string, to: string) =>
  call<Availability>(
    `/t/${enc(slug)}/bookings/${enc(id)}/availability?${new URLSearchParams({ from, to })}`,
  )

/** 員工替顧客改期:同一位員工與服務,系統會寄通知信給顧客 */
export const rescheduleBooking = (slug: string, id: string, start: string) =>
  call<{ id: string; starts_at: string; ends_at: string }>(
    `/t/${enc(slug)}/bookings/${enc(id)}/reschedule`,
    send('POST', { start }),
  )

export const updateBooking = (
  slug: string,
  id: string,
  body: { status?: Exclude<BookingStatus, 'pending' | 'confirmed'>; notes?: string },
) =>
  call<{ id: string; status: BookingStatus; notes: string | null }>(
    `/t/${enc(slug)}/bookings/${enc(id)}`,
    send('PATCH', body),
  )

// ---------- 服務 ----------
export const listServices = (slug: string) =>
  call<ServicePage>(`/t/${enc(slug)}/services?limit=100`)

export interface ServiceInput {
  name: string
  duration_minutes: number
  price_cents: number
}

export const createService = (slug: string, body: ServiceInput) =>
  call<AdminService>(`/t/${enc(slug)}/services`, send('POST', body))

export const updateService = (
  slug: string,
  id: string,
  body: Partial<ServiceInput> & { active?: boolean },
) => call<AdminService>(`/t/${enc(slug)}/services/${enc(id)}`, send('PATCH', body))

export const deleteService = (slug: string, id: string) =>
  call<void>(`/t/${enc(slug)}/services/${enc(id)}`, { method: 'DELETE' })

// ---------- 成員 / 邀請 ----------
export const listMembers = (slug: string) => call<Member[]>(`/t/${enc(slug)}/members`)

export const changeRole = (slug: string, userId: string, role: Exclude<Role, 'owner'>) =>
  call<{ user_id: string; role: Role }>(
    `/t/${enc(slug)}/members/${enc(userId)}`,
    send('PATCH', { role }),
  )

export const removeMember = (slug: string, userId: string) =>
  call<void>(`/t/${enc(slug)}/members/${enc(userId)}`, { method: 'DELETE' })

/** 停用(離職):保留歷史紀錄;還有未來已確認的預約時後端會拒絕 */
export const deactivateMember = (slug: string, userId: string) =>
  call<void>(`/t/${enc(slug)}/members/${enc(userId)}/deactivate`, { method: 'POST' })

export const reactivateMember = (slug: string, userId: string) =>
  call<void>(`/t/${enc(slug)}/members/${enc(userId)}/reactivate`, { method: 'POST' })

export const listInvitations = (slug: string) => call<Invitation[]>(`/t/${enc(slug)}/invitations`)

export const createInvitation = (slug: string, email: string, role: Exclude<Role, 'owner'>) =>
  call<Invitation & { token: string }>(`/t/${enc(slug)}/invitations`, send('POST', { email, role }))

export const revokeInvitation = (slug: string, id: string) =>
  call<void>(`/t/${enc(slug)}/invitations/${enc(id)}`, { method: 'DELETE' })

export const acceptInvitation = (token: string) =>
  call<{ tenant_slug: string }>('/invitations/accept', send('POST', { token }))

// ---------- 員工的服務 / 營業時間 / 休假 ----------
export const getStaffServices = (slug: string, userId: string) =>
  call<{ service_ids: string[] }>(`/t/${enc(slug)}/members/${enc(userId)}/services`)

export const putStaffServices = (slug: string, userId: string, serviceIds: string[]) =>
  call<{ service_ids: string[] }>(
    `/t/${enc(slug)}/members/${enc(userId)}/services`,
    send('PUT', { service_ids: serviceIds }),
  )

export const getWorkingHours = (slug: string, userId: string) =>
  call<{ hours: WorkingHour[] }>(`/t/${enc(slug)}/members/${enc(userId)}/working-hours`)

export const putWorkingHours = (slug: string, userId: string, hours: WorkingHour[]) =>
  call<{ hours: WorkingHour[] }>(
    `/t/${enc(slug)}/members/${enc(userId)}/working-hours`,
    send('PUT', { hours }),
  )

/** 預設只列還沒結束的(進行中與未來);`past` 看已結束的(最近結束的在前) */
export const listTimeOff = (
  slug: string,
  userId: string,
  q: { past?: boolean; offset?: number; limit?: number } = {},
) =>
  call<TimeOffPage>(
    `/t/${enc(slug)}/members/${enc(userId)}/time-off${query({ past: q.past ? 'true' : undefined, offset: q.offset, limit: q.limit })}`,
  )

export const createTimeOff = (
  slug: string,
  userId: string,
  body: { starts_at: string; ends_at: string; reason?: string },
) => call<TimeOff>(`/t/${enc(slug)}/members/${enc(userId)}/time-off`, send('POST', body))

export const deleteTimeOff = (slug: string, id: string) =>
  call<void>(`/t/${enc(slug)}/time-off/${enc(id)}`, { method: 'DELETE' })

// ---------- 稽核 / 方案 ----------
export const listAudit = (
  slug: string,
  q: { action?: string; before?: number; limit?: number } = {},
) => call<AuditPage>(`/t/${enc(slug)}/audit-logs${query({ ...q })}`)

/** 匯出稽核日誌(CSV)。`from` / `to` 是 ISO 時間(含 from、不含 to) */
export const exportAudit = (slug: string, q: { action?: string; from?: string; to?: string }) =>
  apiDownload(`/t/${enc(slug)}/audit-logs/export.csv${query({ ...q })}`)

/** 匯出預約(CSV,管理者以上)。`from` / `to` 是 ISO 時間 */
export const exportBookings = (slug: string, q: { from?: string; to?: string }) =>
  apiDownload(`/t/${enc(slug)}/bookings/export.csv${query({ ...q })}`)

export const getPlan = (slug: string) =>
  call<{ plan: PlanInfo; usage: PlanUsage }>(`/t/${enc(slug)}/plan`)

// ---------- 顧客 ----------
export const listCustomers = (slug: string, q: { q?: string; limit?: number; offset?: number }) =>
  call<Page<Customer>>(`/t/${enc(slug)}/customers${query({ ...q })}`)

export const getCustomer = (slug: string, id: string) =>
  call<CustomerDetail>(`/t/${enc(slug)}/customers/${enc(id)}`)

/** 刪除顧客個資(匿名化;不可還原,僅擁有者)。還有未來 / 待確認的預約時後端會拒絕 */
export const anonymizeCustomer = (slug: string, id: string) =>
  call<void>(`/t/${enc(slug)}/customers/${enc(id)}/anonymize`, { method: 'POST' })

export const getBilling = (slug: string) => call<Billing>(`/t/${enc(slug)}/billing`)

export const startCheckout = (slug: string, plan: string) =>
  call<{ url: string }>(`/t/${enc(slug)}/billing/checkout`, send('POST', { plan }))

export const openPortal = (slug: string) =>
  call<{ url: string }>(`/t/${enc(slug)}/billing/portal`, send('POST', {}))

export const listPlans = () => api<PlanInfo[]>('/plans')

export type { PublicBooking }
