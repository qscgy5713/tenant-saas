import { api, json } from '../api/client'
import type { Availability, BookingStatus, NewBooking, PublicBooking } from '../api/types'
import { getSession } from './session'
import type {
  AdminBooking,
  AdminService,
  AuditPage,
  AuthResponse,
  Billing,
  BookingQuery,
  Invitation,
  Member,
  MyShop,
  Page,
  PlanInfo,
  PlanUsage,
  Role,
  ServicePage,
  TimeOff,
  User,
  WorkingHour,
} from './types'

const enc = encodeURIComponent

/** 帶上登入的 JWT。沒登入就不送 Authorization,由後端回 401(全域處理會導向登入頁) */
function authed(path: string, init: RequestInit = {}) {
  const token = getSession()?.token
  return api<never>(path, {
    ...init,
    headers: { ...(token ? { Authorization: `Bearer ${token}` } : {}), ...init.headers },
  })
}

const call = <T>(path: string, init?: RequestInit) => authed(path, init) as Promise<T>
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

// ---------- 店家 ----------
export const listShops = () => call<MyShop[]>('/tenants')

export const createShop = (body: { slug: string; name: string; timezone: string }) =>
  call<MyShop>('/tenants', send('POST', body))

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

export const listTimeOff = (slug: string, userId: string) =>
  call<TimeOff[]>(`/t/${enc(slug)}/members/${enc(userId)}/time-off`)

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

export const getPlan = (slug: string) =>
  call<{ plan: PlanInfo; usage: PlanUsage }>(`/t/${enc(slug)}/plan`)

export const getBilling = (slug: string) => call<Billing>(`/t/${enc(slug)}/billing`)

export const startCheckout = (slug: string, plan: string) =>
  call<{ url: string }>(`/t/${enc(slug)}/billing/checkout`, send('POST', { plan }))

export const openPortal = (slug: string) =>
  call<{ url: string }>(`/t/${enc(slug)}/billing/portal`, send('POST', {}))

export const listPlans = () => api<PlanInfo[]>('/plans')

export type { PublicBooking }
