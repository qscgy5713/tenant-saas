import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { logoutAllRequest, logoutRequest } from './api'
import { clearSession } from './session'

/**
 * 登出:請後端清掉 HttpOnly cookie、清掉前端的登入狀態與所有快取
 * (避免下一位使用者在同一台電腦看到上一位的資料)。
 * 後端連不上時仍然在前端登出(盡力而為);cookie 最久一小時後自己過期。
 */
export function useLogout() {
  return useSignOut(logoutRequest)
}

/**
 * 登出所有裝置:後端讓此刻之前簽發的登入全部失效。和一般登出不同,**失敗時不能假裝成功** ——
 * 使用者是為了「確保別的裝置登出」才按的,所以由呼叫端處理錯誤,成功了才清前端狀態。
 */
export function useLogoutAll() {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  return async () => {
    await logoutAllRequest()
    clearSession()
    queryClient.clear()
    navigate('/admin/login', { replace: true })
  }
}

function useSignOut(request: () => Promise<unknown>) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  return async () => {
    try {
      await request()
    } catch {
      // 見上:不因為連不上伺服器就讓使用者卡在登入狀態
    }
    clearSession()
    queryClient.clear()
    navigate('/admin/login', { replace: true })
  }
}
