// 對應後端 src/routes/public.rs 的公開 API。欄位名稱與後端 JSON 一致(snake_case)。

export interface Shop {
  name: string
  /** IANA 時區,例如 Asia/Taipei。所有時段都要以這個時區顯示 */
  timezone: string
}

export interface Service {
  id: string
  name: string
  duration_minutes: number
  /** 以「分」為單位的整數(後端沒有幣別欄位,前端暫以新台幣顯示) */
  price_cents: number
}

export interface Staff {
  id: string
  name: string
}

export interface Slot {
  staff_id: string
  /** UTC ISO 時間 */
  start: string
  end: string
}

export interface Availability {
  timezone: string
  slots: Slot[]
}

export type BookingStatus = 'pending' | 'confirmed' | 'cancelled' | 'completed' | 'no_show'

export interface NewBooking {
  service_id: string
  /** 不指定員工就省略,由後端挑有空的人 */
  staff_id?: string
  start: string
  customer: { name: string; email: string; phone?: string }
}

export interface BookingRequested {
  id: string
  status: BookingStatus
  starts_at: string
  ends_at: string
  message: string
}

export interface PublicBooking {
  id: string
  status: BookingStatus
  starts_at: string
  ends_at: string
  shop_name: string
  shop_slug: string
  timezone: string
  service_id: string
  service_name: string
  staff_id: string
  staff_name: string
  customer_name: string
}
