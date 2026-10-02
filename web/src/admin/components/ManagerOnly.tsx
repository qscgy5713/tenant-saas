import type { ReactNode } from 'react'
import { Link } from 'react-router-dom'
import { useShop } from '../ShopContext'

/** 只有擁有者與管理者能看的頁面。這只是讓員工看到清楚的說明,真正的權限檢查在後端(API 一律會回 403) */
export function ManagerOnly({ children }: { children: ReactNode }) {
  const { canManage, slug } = useShop()
  if (canManage) return children
  return (
    <div className="state">
      <h1>沒有權限</h1>
      <p>這個頁面只有擁有者與管理者可以查看。</p>
      <Link className="btn btn-secondary" to={`/admin/${slug}`}>
        回到總覽
      </Link>
    </div>
  )
}
