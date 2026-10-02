import type { ReactNode } from 'react'
import { Link } from 'react-router-dom'
import { useTitle } from '../lib/useTitle'
import { CalendarIcon } from './Icons'

export function Shell({
  title,
  shopName,
  shopSlug,
  children,
  wide = false,
}: {
  /** 瀏覽器分頁標題 */
  title?: string
  shopName?: string
  shopSlug?: string
  children: ReactNode
  /** 服務列表之類的單欄頁面用較窄的版面 */
  wide?: boolean
}) {
  useTitle(title ? `${title}${shopName ? ` · ${shopName}` : ''}` : shopName)
  return (
    <div className="shell">
      <header className="topbar">
        <div className={`topbar-inner ${wide ? '' : 'topbar-inner-narrow'}`}>
          {shopName && shopSlug ? (
            <Link to={`/s/${shopSlug}`} className="brand">
              <span className="brand-mark">
                <CalendarIcon width={18} height={18} />
              </span>
              <span className="brand-name">{shopName}</span>
            </Link>
          ) : (
            <span className="brand">
              <span className="brand-mark">
                <CalendarIcon width={18} height={18} />
              </span>
              <span className="brand-name">{shopName ?? '線上預約'}</span>
            </span>
          )}
          <span className="topbar-tag">線上預約</span>
        </div>
      </header>
      <main className={`container ${wide ? '' : 'container-narrow'}`}>{children}</main>
    </div>
  )
}
