import { useMutation } from '@tanstack/react-query'
import { useState } from 'react'
import { Link, useLocation, useNavigate } from 'react-router-dom'
import { ApiError } from '../../api/client'
import { Notice } from '../../components/States'
import { acceptInvitation } from '../api'
import { AuthCard } from '../components/AuthCard'
import { useLogout } from '../logout'
import { useSession } from '../session'

/** 邀請信的連結:`/invitations/accept#token=…`。token 放在 # 後面,不會送到伺服器、不會出現在存取紀錄或 Referer */
function tokenFrom(hash: string, search: string): string | null {
  const fromHash = new URLSearchParams(hash.replace(/^#/, '')).get('token')
  return fromHash ?? new URLSearchParams(search).get('token') // 相容舊格式 ?token=
}

export function AcceptInvitationPage() {
  const location = useLocation()
  const navigate = useNavigate()
  const session = useSession()
  const logout = useLogout()
  // 只在第一次渲染時讀取,之後登入、導頁都不再依賴網址
  const [token] = useState(() => tokenFrom(location.hash, location.search))
  const here = `${location.pathname}${location.hash}${location.search}`

  const mutation = useMutation({
    mutationFn: () => acceptInvitation(token!),
    onSuccess: (r) => navigate(`/admin/${r.tenant_slug}`, { replace: true }),
  })

  if (!token || token.length < 16) {
    return (
      <AuthCard title="邀請連結不完整">
        <p>請使用邀請信中的完整連結。</p>
        <Link className="btn btn-secondary" to="/admin">
          前往後台
        </Link>
      </AuthCard>
    )
  }

  if (!session) {
    return (
      <AuthCard title="加入團隊" subtitle="請先使用被邀請的 Email 登入或註冊,再回到這裡接受邀請。">
        <div className="actions">
          <Link className="btn btn-primary" to="/admin/login" state={{ from: here }}>
            登入
          </Link>
          <Link className="btn btn-secondary" to="/admin/register" state={{ from: here }}>
            註冊
          </Link>
        </div>
      </AuthCard>
    )
  }

  const error = mutation.error
  // 後端對「連結無效 / 過期 / 已使用 / Email 不符」一律回 404,不洩漏是哪一種
  const message =
    error instanceof ApiError
      ? error.status === 404
        ? '這個邀請無效:可能已過期、已被使用,或你目前登入的 Email 與被邀請的不同。'
        : error.message
      : error
        ? '操作失敗,請再試一次。'
        : null

  return (
    <AuthCard title="加入團隊" subtitle="接受邀請後,你就能進入這家店的後台。">
      <p className="muted small">
        目前登入:<strong>{session.user.email}</strong>(必須與被邀請的 Email 相同)
      </p>
      {message && <Notice tone="error">{message}</Notice>}
      <div className="actions">
        <button
          type="button"
          className="btn btn-primary"
          onClick={() => mutation.mutate()}
          disabled={mutation.isPending}
        >
          {mutation.isPending ? '處理中…' : '接受邀請'}
        </button>
        <button type="button" className="btn btn-secondary" onClick={logout}>
          換個帳號
        </button>
      </div>
    </AuthCard>
  )
}
