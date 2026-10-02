import { lazy, Suspense } from 'react'
import { Route, Routes } from 'react-router-dom'
import { Loading } from './components/States'
import { BookPage } from './pages/BookPage'
import { HomePage, NotFoundPage } from './pages/HomePage'
import { ManagePage } from './pages/ManagePage'
import { ShopPage } from './pages/ShopPage'

// 後台是另一包程式碼,進入 /admin 或邀請頁才下載
const AdminApp = lazy(() => import('./admin/AdminApp'))
const InvitationEntry = lazy(() => import('./admin/InvitationEntry'))

export function App() {
  return (
    <Routes>
      <Route path="/" element={<HomePage />} />
      <Route path="/s/:slug" element={<ShopPage />} />
      <Route path="/s/:slug/book/:serviceId" element={<BookPage />} />
      {/* 後端寄出的信件連結:`${PUBLIC_BASE_URL}/bookings/{token}` */}
      <Route path="/bookings/:token" element={<ManagePage />} />
      <Route
        path="/admin/*"
        element={
          <Suspense fallback={<Loading />}>
            <AdminApp />
          </Suspense>
        }
      />
      <Route
        path="/invitations/accept"
        element={
          <Suspense fallback={<Loading />}>
            <InvitationEntry />
          </Suspense>
        }
      />
      <Route path="*" element={<NotFoundPage />} />
    </Routes>
  )
}
