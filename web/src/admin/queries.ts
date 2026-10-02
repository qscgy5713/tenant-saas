import { useQuery } from '@tanstack/react-query'
import { listBookings, listMembers } from './api'
import type { BookingQuery } from './types'

export const bookingsKey = (slug: string) => ['admin', 'bookings', slug] as const

/** 預約列表。資料會被操作改變,所以快取很短 */
export const useBookings = (slug: string, params: BookingQuery, enabled = true) =>
  useQuery({
    queryKey: [...bookingsKey(slug), params],
    queryFn: () => listBookings(slug, params),
    staleTime: 10_000,
    enabled,
  })

export const useMembers = (slug: string) =>
  useQuery({
    queryKey: ['admin', 'members', slug],
    queryFn: () => listMembers(slug),
    staleTime: 30_000,
  })
