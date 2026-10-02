import { Navigate, Outlet, useLocation } from 'react-router-dom'
import { useSession } from './session'

export function RequireAuth() {
  const session = useSession()
  const location = useLocation()
  if (!session) {
    const from = `${location.pathname}${location.search}${location.hash}`
    return <Navigate to="/admin/login" state={{ from }} replace />
  }
  return <Outlet />
}
