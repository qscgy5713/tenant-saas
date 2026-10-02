import type { ReactNode } from 'react'
import { Link } from 'react-router-dom'
import { useTitle } from '../lib/useTitle'

export function Shell({
  title,
  shopName,
  shopSlug,
  children,
}: {
  /** 瀏覽器分頁標題 */
  title?: string
  shopName?: string
  shopSlug?: string
  children: ReactNode
}) {
  useTitle(title ? `${title}${shopName ? ` · ${shopName}` : ''}` : shopName)
  return (
    <div className="shell">
      <header className="topbar">
        <div className="topbar-inner">
          {shopName && shopSlug ? (
            <Link to={`/s/${shopSlug}`} className="brand">
              {shopName}
            </Link>
          ) : (
            <span className="brand">{shopName ?? '線上預約'}</span>
          )}
          <span className="topbar-tag">線上預約</span>
        </div>
      </header>
      <main className="container">{children}</main>
    </div>
  )
}
