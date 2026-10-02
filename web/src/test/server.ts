import { HttpResponse, http } from 'msw'
import { setupServer } from 'msw/node'
import type { PublicBooking, Service, Slot, Staff } from '../api/types'
import { addDays, todayYmd } from '../lib/time'

export const API = 'http://api.test'
export const TZ = 'Asia/Taipei'
/** 週一 2026-10-05 10:00(台北) */
export const NOW = new Date('2026-10-05T02:00:00Z')

export const server = setupServer()

export const SERVICE: Service = {
  id: 'svc-cut',
  name: '剪髮',
  duration_minutes: 60,
  price_cents: 50000,
}
export const STAFF_A: Staff = { id: 'staff-a', name: '林美玲' }
export const STAFF_B: Staff = { id: 'staff-b', name: '陳小安' }

export const error = (status: number, message: string, requestId?: string) =>
  HttpResponse.json(
    { error: message },
    { status, headers: requestId ? { 'x-request-id': requestId } : {} },
  )

/** 店家當地(台北)某日的 hh:mm → UTC ISO */
export function taipei(ymd: string, hhmm: string): string {
  return new Date(`${ymd}T${hhmm}:00+08:00`).toISOString()
}

/** 週一到週六營業,每天 10:00 / 10:30 / 14:00 可預約 */
export function standardSlots(from: string, to: string, staffIds: string[] = [STAFF_A.id]): Slot[] {
  const slots: Slot[] = []
  for (let day = from; day <= to; day = addDays(day, 1)) {
    if (new Date(`${day}T12:00:00Z`).getUTCDay() === 0) continue // 週日不營業
    for (const hhmm of ['10:00', '10:30', '14:00']) {
      for (const staff_id of staffIds) {
        const start = taipei(day, hhmm)
        slots.push({ staff_id, start, end: new Date(Date.parse(start) + 3_600_000).toISOString() })
      }
    }
  }
  return slots
}

export const today = () => todayYmd(TZ, NOW)

export interface ShopMocks {
  services?: Service[]
  staff?: Staff[]
  /** 自訂時段;預設為 standardSlots */
  slots?: (q: { from: string; to: string; staffId: string | null }) => Slot[]
  availabilityCalls?: URLSearchParams[]
}

/** 註冊一組店家 demo-salon 的公開 API */
export function mockShop(m: ShopMocks = {}) {
  const services = m.services ?? [SERVICE]
  const staff = m.staff ?? [STAFF_A]
  server.use(
    http.get(`${API}/public/shops/demo-salon`, () =>
      HttpResponse.json({ name: '森林系髮廊', timezone: TZ }),
    ),
    http.get(`${API}/public/shops/demo-salon/services`, () => HttpResponse.json(services)),
    http.get(`${API}/public/shops/demo-salon/services/:id/staff`, () => HttpResponse.json(staff)),
    http.get(`${API}/public/shops/demo-salon/availability`, ({ request }) => {
      const q = new URL(request.url).searchParams
      m.availabilityCalls?.push(q)
      const from = q.get('from')!
      const to = q.get('to')!
      const staffId = q.get('staff_id')
      const slots = m.slots
        ? m.slots({ from, to, staffId })
        : standardSlots(from, to, staffId ? [staffId] : staff.map((s) => s.id).slice(0, 1))
      return HttpResponse.json({ timezone: TZ, slots })
    }),
  )
}

export function booking(overrides: Partial<PublicBooking> = {}): PublicBooking {
  return {
    id: 'b1',
    status: 'confirmed',
    starts_at: taipei('2026-10-08', '14:00'),
    ends_at: taipei('2026-10-08', '15:00'),
    shop_name: '森林系髮廊',
    shop_slug: 'demo-salon',
    timezone: TZ,
    service_id: SERVICE.id,
    service_name: SERVICE.name,
    staff_id: STAFF_A.id,
    staff_name: STAFF_A.name,
    customer_name: '王小明',
    ...overrides,
  }
}
