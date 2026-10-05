import { Navigate, Outlet, useLocation } from 'react-router-dom'
import { Loading } from '../components/States'
import { useSession, useSessionStatus } from './session'

export function RequireAuth() {
  const session = useSession()
  const status = useSessionStatus()
  const location = useLocation()
  // 重新整理後還在向後端確認登入狀態:先等,不然會閃一下登入頁
  if (status === 'loading') return <Loading />
  if (!session) {
    const from = `${location.pathname}${location.search}${location.hash}`
    return <Navigate to="/admin/login" state={{ from }} replace />
  }
  return <Outlet />
}
