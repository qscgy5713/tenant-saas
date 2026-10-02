import { MutationCache, QueryCache, QueryClient } from '@tanstack/react-query'
import { clearSession, getSession } from './admin/session'
import { ApiError } from './api/client'
import { shouldRetry } from './api/queries'

/** 後端說「未登入或憑證無效」(401):登入已過期或被撤銷,清掉登入狀態,頁面會自動導向登入頁 */
export function onApiError(error: unknown) {
  if (error instanceof ApiError && error.status === 401 && getSession()) clearSession()
}

/** 正式環境與測試共用。測試傳 `testing: true` 關掉重試,讓失敗立刻呈現 */
export function createQueryClient({ testing = false }: { testing?: boolean } = {}) {
  return new QueryClient({
    queryCache: new QueryCache({ onError: onApiError }),
    mutationCache: new MutationCache({ onError: onApiError }),
    defaultOptions: {
      // 測試只關掉重試。不要動 gcTime:設成 0 會讓卸載後的快取立刻被回收,
      // 「登出後快取是空的」就算沒清也成立,測試形同虛設
      queries: testing ? { retry: false } : { retry: shouldRetry, refetchOnWindowFocus: false },
      // 預約 / 確認 / 取消這類有副作用的請求絕不自動重試:重送可能造成重複操作
      mutations: { retry: false },
    },
  })
}
