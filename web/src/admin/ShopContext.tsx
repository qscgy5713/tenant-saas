import { createContext, useContext } from 'react'
import type { Role, ShopDetail, User } from './types'

export interface ShopCtx {
  slug: string
  shop: ShopDetail
  user: User
  role: Role
  /** owner / manager:可以管理服務、成員、查看稽核與方案 */
  canManage: boolean
  isOwner: boolean
}

export const ShopContext = createContext<ShopCtx | null>(null)

export function useShop(): ShopCtx {
  const ctx = useContext(ShopContext)
  if (!ctx) throw new Error('useShop 必須在 ShopLayout 內使用')
  return ctx
}

export const ROLE_LABEL: Record<Role, string> = {
  owner: '擁有者',
  manager: '管理者',
  staff: '員工',
}
