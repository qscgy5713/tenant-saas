// 對應後端「需登入」的 API(src/routes/*.rs)。欄位名稱與後端 JSON 一致。
import type { BookingStatus } from '../api/types'

export type Role = 'owner' | 'manager' | 'staff'

export interface User {
  id: string
  email: string
  name: string
  /** 沒驗證 Email 不能建立店家 */
  email_verified: boolean
}

/** 登入 / 註冊的回應。後端也會在 body 附上 token(給非瀏覽器客戶端),網頁刻意不使用,只靠 cookie */
export interface AuthResponse {
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

/** 店家自己的資料(`/t/{slug}/me`):比店家列表多了資料保留與刪除狀態 */
export interface ShopDetail extends MyShop {
  /** 顧客個資在最後一筆預約之後保留幾天,到期自動匿名化 */
  customer_retention_days: number
  /** 已申請刪除時,預定永久刪除的時間;沒有申請時為 null */
  deletion_scheduled_at: string | null
  /** 顧客預約頁上顯示的店家資訊(都是選填) */
  description: string | null
  address: string | null
  phone: string | null
}

/** 帳號層級的安全事件(只有本人看得到) */
export interface AccountEvent {
  kind:
    | 'registered'
    | 'login'
    | 'login_failed'
    | 'locked'
    | 'logout_all'
    | 'password_reset'
    | 'email_verified'
  created_at: string
}

export interface Member {
  user_id: string
  name: string
  email: string
  role: Role
  /** false = 已停用(離職):進不了這家店、不能被預約,歷史紀錄保留 */
  active: boolean
}

export interface AdminService {
  id: string
  name: string
  duration_minutes: number
  /** 服務結束後員工的整理時間(分鐘),顧客看不到 */
  buffer_minutes: number
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

/** 休假清單的一頁。`has_more` 為 true 時後面還有 */
export interface TimeOffPage {
  items: TimeOff[]
  limit: number
  offset: number
  has_more: boolean
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

/** GET /t/{slug}/billing(只有店主能看) */
export interface Billing {
  enabled: boolean
  /** 可以線上訂閱的方案 id */
  purchasable: string[]
  subscription: {
    status: string
    current_period_end: string | null
    cancel_at_period_end: boolean
  } | null
  can_checkout: boolean
  can_manage: boolean
}

export interface Customer {
  id: string
  name: string
  email: string
  phone: string | null
  /** 已確認 + 已完成 + 未到 */
  bookings: number
  completed: number
  no_shows: number
  last_at: string | null
  next_at: string | null
}

export interface CustomerHistoryItem {
  id: string
  status: BookingStatus
  starts_at: string
  ends_at: string
  notes: string | null
  service_name: string
  staff_name: string
}

export interface CustomerDetail extends Customer {
  history: CustomerHistoryItem[]
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
