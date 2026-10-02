import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { clearSession } from './session'

/** 登出:清掉登入狀態與所有快取(避免下一位使用者在同一台電腦看到上一位的資料) */
export function useLogout() {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  return () => {
    clearSession()
    queryClient.clear()
    navigate('/admin/login', { replace: true })
  }
}
