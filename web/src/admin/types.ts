// 對應後端「需登入」的 API(src/routes/*.rs)。欄位名稱與後端 JSON 一致。
import type { BookingStatus } from '../api/types'

export type Role = 'owner' | 'manager' | 'staff'

export interface User {
  id: string
  email: string
  name: string
}

export interface AuthResponse {
  token: string
  user: User
}

/** GET /tenants 與 /t/{slug}/me:我在某家店的身分 */
export interface MyShop {
  id: string
  slug: string
  name: string
  timezone: string
  role: Role
}

export interface Member {
  user_id: string
  name: string
  email: string
  role: Role
}

export interface AdminService {
  id: string
  name: string
  duration_minutes: number
  price_cents: number
  active: boolean
}

export interface Page<T> {
  items: T[]
  limit: number
  offset: number
}

export interface ServicePage extends Page<AdminService> {
  total: number
}

export interface AdminBooking {
  id: string
  status: BookingStatus
  starts_at: string
  ends_at: string
  notes: string | null
  service_id: string
  service_name: string
  staff_id: string
  staff_name: string
  customer_name: string
  customer_email: string
  customer_phone: string | null
}

export interface BookingQuery {
  from?: string
  to?: string
  status?: BookingStatus
  staffId?: string
  limit?: number
  offset?: number
}

export interface Invitation {
  id: string
  email: string
  role: Role
  expires_at: string
  created_at: string
}

export interface WorkingHour {
  /** 0 = 週日 … 6 = 週六 */
  weekday: number
  /** "HH:MM",店家當地時間 */
  start: string
  end: string
}

export interface TimeOff {
  id: string
  user_id: string
  starts_at: string
  ends_at: string
  reason: string | null
}

export interface AuditEntry {
  id: number
  created_at: string
  actor_type: 'user' | 'customer' | 'system'
  actor_user_id: string | null
  actor_name: string | null
  action: string
  entity_type: string
  entity_id: string | null
  detail: Record<string, unknown>
}

export interface AuditPage {
  items: AuditEntry[]
  next_before: number | null
}

export interface PlanInfo {
  id: string
  name: string
  price_cents: number
  max_staff: number | null
  max_services: number | null
  max_bookings_per_month: number | null
}

export interface PlanUsage {
  staff: number
  pending_invitations: number
  services: number
  bookings_this_month: number
}
