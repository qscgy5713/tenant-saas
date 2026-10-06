import { Route, Routes } from 'react-router-dom'
import './admin.css'
import { RequireAuth } from './auth'
import { ManagerOnly } from './components/ManagerOnly'
import { AuditPage } from './pages/AuditPage'
import { BookingsPage } from './pages/BookingsPage'
import { LoginPage } from './pages/LoginPage'
import { OverviewPage } from './pages/OverviewPage'
import { PlanPage } from './pages/PlanPage'
import { RegisterPage } from './pages/RegisterPage'
import { MemberPage } from './pages/MemberPage'
import { ServicesPage } from './pages/ServicesPage'
import { ShopsPage } from './pages/ShopsPage'
import { TeamPage } from './pages/TeamPage'
import { ForgotPasswordPage } from './pages/ForgotPasswordPage'
import { ResetPasswordPage } from './pages/ResetPasswordPage'
import { VerifyEmailPage } from './pages/VerifyEmailPage'
import { CustomersPage } from './pages/CustomersPage'
import { SettingsPage } from './pages/SettingsPage'
import { ShopLayout } from './ShopLayout'
import { useClearCacheOnLogout } from './useClearCacheOnLogout'

/** 掛在 /admin/*,路徑相對於 /admin。用 React.lazy 載入,一般顧客的預約頁不會下載這一整包程式 */
export default function AdminApp() {
  useClearCacheOnLogout()
  return (
    <Routes>
      <Route path="login" element={<LoginPage />} />
      <Route path="register" element={<RegisterPage />} />
      <Route path="forgot" element={<ForgotPasswordPage />} />
      <Route path="reset" element={<ResetPasswordPage />} />
      <Route path="verify" element={<VerifyEmailPage />} />
      <Route element={<RequireAuth />}>
        <Route index element={<ShopsPage />} />
        <Route path=":slug" element={<ShopLayout />}>
          <Route index element={<OverviewPage />} />
          <Route path="bookings" element={<BookingsPage />} />
          <Route path="services" element={<ServicesPage />} />
          <Route path="team" element={<TeamPage />} />
          <Route path="team/:userId" element={<MemberPage />} />
          <Route
            path="customers"
            element={
              <ManagerOnly>
                <CustomersPage />
              </ManagerOnly>
            }
          />
          <Route
            path="audit"
            element={
              <ManagerOnly>
                <AuditPage />
              </ManagerOnly>
            }
          />
          <Route path="settings" element={<SettingsPage />} />
          <Route
            path="plan"
            element={
              <ManagerOnly>
                <PlanPage />
              </ManagerOnly>
            }
          />
        </Route>
      </Route>
    </Routes>
  )
}
