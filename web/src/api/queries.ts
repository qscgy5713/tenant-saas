import { useQuery } from '@tanstack/react-query'
import { ApiError } from './client'
import { getAvailability, getBooking, getServices, getShop, getStaff } from './public'
import type { Availability } from './types'

/** 4xx 重試同樣的請求不會有不同結果,不要重試;網路 / 5xx 最多重試兩次 */
export function shouldRetry(failureCount: number, error: unknown): boolean {
  if (error instanceof ApiError && error.isClientError) return false
  return failureCount < 2
}

const MINUTE = 60_000

export const useShop = (slug: string) =>
  useQuery({ queryKey: ['shop', slug], queryFn: () => getShop(slug), staleTime: 5 * MINUTE })

export const useServices = (slug: string) =>
  useQuery({ queryKey: ['services', slug], queryFn: () => getServices(slug), staleTime: MINUTE })

export const useStaff = (slug: string, serviceId: string) =>
  useQuery({
    queryKey: ['staff', slug, serviceId],
    queryFn: () => getStaff(slug, serviceId),
    staleTime: MINUTE,
  })

/**
 * 可預約時段的「來源」:查詢鍵 + 取資料的函式。
 * 店家公開的、顧客依自己的預約、員工依某筆預約,三種來源的資料不同(後兩者會排除預約自己),
 * 但 SlotPicker 的畫面完全一樣。
 */
export interface SlotSource {
  key: readonly unknown[]
  fetch: (from: string, to: string) => Promise<Availability>
}

export const shopSlotSource = (
  slug: string,
  serviceId: string,
  staffId: string | null,
): SlotSource => ({
  key: ['availability', slug, serviceId, staffId],
  fetch: (from, to) =>
    getAvailability(slug, { serviceId, from, to, staffId: staffId ?? undefined }),
})

/** 可預約時段變動很快(別人隨時會訂走),所以快取很短 */
export const useSlots = (source: SlotSource, from: string, to: string) =>
  useQuery({
    queryKey: [...source.key, from, to],
    queryFn: () => source.fetch(from, to),
    staleTime: 15_000,
  })

export const useBooking = (token: string) =>
  useQuery({ queryKey: ['booking', token], queryFn: () => getBooking(token), staleTime: 0 })
