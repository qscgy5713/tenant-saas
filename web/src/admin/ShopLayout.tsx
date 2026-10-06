import { useQuery } from '@tanstack/react-query'
import { useMemo } from 'react'
import { Link, NavLink, Outlet, useParams } from 'react-router-dom'
import { ApiError } from '../api/client'
import { Avatar } from '../components/Avatar'
import {
  CardIcon,
  UserIcon,
  SettingsIcon,
  ExternalLinkIcon,
  HomeIcon,
  ListIcon,
  LogOutIcon,
  ShieldIcon,
  SwitchIcon,
  TagIcon,
  UsersIcon,
} from '../components/Icons'
import { ErrorState, Loading } from '../components/States'
import { formatDateTime } from '../lib/time'
import { useTitle } from '../lib/useTitle'
import { getShop } from './api'
import { useLogout } from './logout'
import { ROLE_LABEL, ShopContext, type ShopCtx } from './ShopContext'
import { useSession } from './session'

interface NavItem {
  to: string
  label: string
  icon: React.ReactNode
  /** 只有 owner / manager 看得到(後端本來就會擋,這裡只是不顯示用不到的入口) */
  managerOnly?: boolean
  /** 只有店主看得到 */
  ownerOnly?: boolean
  end?: boolean
}

const NAV: NavItem[] = [
  { to: '', label: '總覽', icon: <HomeIcon />, end: true },
  { to: 'bookings', label: '預約', icon: <ListIcon /> },
  { to: 'services', label: '服務', icon: <TagIcon /> },
  { to: 'team', label: '團隊', icon: <UsersIcon /> },
  { to: 'customers', label: '顧客', icon: <UserIcon />, managerOnly: true },
  { to: 'audit', label: '稽核', icon: <ShieldIcon />, managerOnly: true },
  { to: 'plan', label: '方案', icon: <CardIcon />, managerOnly: true },
  { to: 'settings', label: '設定', icon: <SettingsIcon />, ownerOnly: true },
]

export function ShopLayout() {
  const { slug = '' } = useParams()
  const session = useSession()
  const logout = useLogout()
  const shop = useQuery({ queryKey: ['admin', 'shop', slug], queryFn: () => getShop(slug) })

  const ctx = useMemo<ShopCtx | null>(() => {
    if (!shop.data || !session) return null
    const role = shop.data.role
    return {
      slug,
      shop: shop.data,
      user: session.user,
      role,
      canManage: role === 'owner' || role === 'manager',
      isOwner: role === 'owner',
    }
  }, [shop.data, session, slug])

  useTitle(shop.data ? `${shop.data.name} · 店家後台` : '店家後台')

  if (shop.isPending) {
    return (
      <div className="admin-boot">
        <Loading />
      </div>
    )
  }
  if (shop.isError || !ctx) {
    const notFound = shop.error instanceof ApiError && shop.error.status === 404
    return (
      <div className="admin-boot">
        <ErrorState
          error={shop.error}
          title={notFound ? '找不到這家店' : '發生問題'}
          onRetry={() => shop.refetch()}
        >
          {notFound && <p className="muted">這家店不存在,或你不是它的成員。</p>}
          <Link className="btn btn-secondary" to="/admin">
            回到我的店家
          </Link>
        </ErrorState>
      </div>
    )
  }

  const items = NAV.filter(
    (item) => (!item.managerOnly || ctx.canManage) && (!item.ownerOnly || ctx.isOwner),
  )

  return (
    <ShopContext.Provider value={ctx}>
      <div className="admin">
        <aside className="side">
          <div className="side-shop">
            <Avatar name={ctx.shop.name} size="md" />
            <div className="side-shop-text">
              <strong>{ctx.shop.name}</strong>
              <span className="muted small">{ROLE_LABEL[ctx.role]}</span>
            </div>
          </div>
          <nav aria-label="後台選單" className="side-nav">
            {items.map((item) => (
              <NavLink
                key={item.label}
                to={item.to}
                end={item.end}
                className={({ isActive }) => `side-link ${isActive ? 'side-link-on' : ''}`}
              >
                {item.icon}
                <span>{item.label}</span>
              </NavLink>
            ))}
          </nav>
          <div className="side-foot">
            <a className="side-link" href={`/s/${slug}`} target="_blank" rel="noreferrer">
              <ExternalLinkIcon />
              <span>查看預約頁</span>
            </a>
            <Link className="side-link" to="/admin">
              <SwitchIcon />
              <span>切換店家</span>
            </Link>
            <button type="button" className="side-link side-link-btn" onClick={logout}>
              <LogOutIcon />
              <span>登出</span>
            </button>
            <p className="side-user muted small">{session?.user.name}</p>
          </div>
        </aside>

        <div className="admin-main">
          <header className="admin-top">
            <Avatar name={ctx.shop.name} size="sm" />
            <strong>{ctx.shop.name}</strong>
            <span className="grow" />
            <a
              className="icon-btn icon-btn-sm"
              href={`/s/${slug}`}
              target="_blank"
              rel="noreferrer"
              aria-label="查看預約頁"
            >
              <ExternalLinkIcon width={16} height={16} />
            </a>
            <button
              type="button"
              className="icon-btn icon-btn-sm"
              onClick={logout}
              aria-label="登出"
            >
              <LogOutIcon width={16} height={16} />
            </button>
          </header>
          {ctx.shop.deletion_scheduled_at && (
            <div className="notice notice-warning deletion-banner" role="status">
              這家店已申請刪除:公開預約頁已關閉,
              {formatDateTime(ctx.shop.deletion_scheduled_at, ctx.shop.timezone)}
              之後會連同所有資料永久刪除。
              {ctx.isOwner ? (
                <>
                  {' '}
                  <Link to={`/admin/${slug}/settings`}>到設定頁取消刪除</Link>
                </>
              ) : (
                ' 只有擁有者可以取消。'
              )}
            </div>
          )}
          <main className="admin-content">
            <Outlet />
          </main>
        </div>

        <nav aria-label="後台選單(手機)" className="tabbar">
          {items.map((item) => (
            <NavLink
              key={item.label}
              to={item.to}
              end={item.end}
              className={({ isActive }) => `tab ${isActive ? 'tab-on' : ''}`}
            >
              {item.icon}
              <span>{item.label}</span>
            </NavLink>
          ))}
        </nav>
      </div>
    </ShopContext.Provider>
  )
}
