import { api, json } from './client'
import type {
  Availability,
  BookingRequested,
  NewBooking,
  PublicBooking,
  Service,
  Shop,
  Staff,
} from './types'

const enc = encodeURIComponent

export const getShop = (slug: string) => api<Shop>(`/public/shops/${enc(slug)}`)

export const getServices = (slug: string) => api<Service[]>(`/public/shops/${enc(slug)}/services`)

export const getStaff = (slug: string, serviceId: string) =>
  api<Staff[]>(`/public/shops/${enc(slug)}/services/${enc(serviceId)}/staff`)

export interface AvailabilityQuery {
  serviceId: string
  /** 店家當地日期 YYYY-MM-DD(含) */
  from: string
  to: string
  staffId?: string
}

export function getAvailability(slug: string, q: AvailabilityQuery) {
  const params = new URLSearchParams({ service_id: q.serviceId, from: q.from, to: q.to })
  if (q.staffId) params.set('staff_id', q.staffId)
  return api<Availability>(`/public/shops/${enc(slug)}/availability?${params}`)
}

/** 這筆預約改期時可選的時段:同一位員工與服務,而且排除這筆預約自己(自己的時段不是忙碌) */
export const getBookingAvailability = (token: string, from: string, to: string) =>
  api<Availability>(
    `/public/bookings/${enc(token)}/availability?${new URLSearchParams({ from, to })}`,
  )

export const createBooking = (slug: string, body: NewBooking) =>
  api<BookingRequested>(`/public/shops/${enc(slug)}/bookings`, json(body))

export const getBooking = (token: string) => api<PublicBooking>(`/public/bookings/${enc(token)}`)

export const confirmBooking = (token: string) =>
  api<PublicBooking>(`/public/bookings/${enc(token)}/confirm`, { method: 'POST' })

export const cancelBooking = (token: string) =>
  api<PublicBooking>(`/public/bookings/${enc(token)}/cancel`, { method: 'POST' })

export const rescheduleBooking = (token: string, start: string) =>
  api<PublicBooking>(`/public/bookings/${enc(token)}/reschedule`, json({ start }))
