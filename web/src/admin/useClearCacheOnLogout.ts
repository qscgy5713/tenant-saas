import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useSession } from './session'

/** 登入狀態消失(登出、過期、被後端判定無效)時清掉所有快取,避免下一位使用者看到上一位的資料 */
export function useClearCacheOnLogout() {
  const session = useSession()
  const queryClient = useQueryClient()
  useEffect(() => {
    if (!session) queryClient.clear()
  }, [session, queryClient])
}
