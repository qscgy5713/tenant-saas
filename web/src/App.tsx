import { Route, Routes } from 'react-router-dom'
import { BookPage } from './pages/BookPage'
import { HomePage, NotFoundPage } from './pages/HomePage'
import { ManagePage } from './pages/ManagePage'
import { ShopPage } from './pages/ShopPage'

export function App() {
  return (
    <Routes>
      <Route path="/" element={<HomePage />} />
      <Route path="/s/:slug" element={<ShopPage />} />
      <Route path="/s/:slug/book/:serviceId" element={<BookPage />} />
      {/* 後端寄出的信件連結:`${PUBLIC_BASE_URL}/bookings/{token}` */}
      <Route path="/bookings/:token" element={<ManagePage />} />
      <Route path="*" element={<NotFoundPage />} />
    </Routes>
  )
}
