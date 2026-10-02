import { useQuery } from '@tanstack/react-query'
import { listBookings, listMembers } from './api'
import type { AdminBooking, BookingQuery } from './types'

export const bookingsKey = (slug: string) => ['admin', 'bookings', slug] as const

/** 預約列表。資料會被操作改變,所以快取很短 */
export const useBookings = (slug: string, params: BookingQuery, enabled = true) =>
  useQuery({
    queryKey: [...bookingsKey(slug), params],
    queryFn: () => listBookings(slug, params),
    staleTime: 10_000,
    enabled,
  })

/** 日曆要一次畫整週,不能只看第一頁:一頁一頁抓到抓完(最多 PAGES 頁,避免失控) */
const CAL_PAGE = 100
const CAL_MAX_PAGES = 5
export const useAllBookings = (slug: string, params: BookingQuery, enabled = true) =>
  useQuery({
    queryKey: [...bookingsKey(slug), 'all', params],
    queryFn: async () => {
      const items: AdminBooking[] = []
      let truncated = false
      for (let page = 0; page < CAL_MAX_PAGES; page++) {
        const r = await listBookings(slug, { ...params, limit: CAL_PAGE, offset: page * CAL_PAGE })
        items.push(...r.items)
        if (r.items.length < CAL_PAGE) return { items, truncated }
      }
      truncated = true
      return { items, truncated }
    },
    staleTime: 10_000,
    enabled,
  })

export const useMembers = (slug: string) =>
  useQuery({
    queryKey: ['admin', 'members', slug],
    queryFn: () => listMembers(slug),
    staleTime: 30_000,
  })
