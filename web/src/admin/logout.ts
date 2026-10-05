import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { logoutRequest } from './api'
import { clearSession } from './session'

/**
 * 登出:請後端清掉 HttpOnly cookie、清掉前端的登入狀態與所有快取
 * (避免下一位使用者在同一台電腦看到上一位的資料)。
 * 後端連不上時仍然在前端登出(盡力而為);cookie 最久一小時後自己過期。
 */
export function useLogout() {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  return async () => {
    try {
      await logoutRequest()
    } catch {
      // 見上:不因為連不上伺服器就讓使用者卡在登入狀態
    }
    clearSession()
    queryClient.clear()
    navigate('/admin/login', { replace: true })
  }
}
