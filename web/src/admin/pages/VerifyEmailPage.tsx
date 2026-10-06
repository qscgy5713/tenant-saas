import { useMutation } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { Link, useLocation } from 'react-router-dom'
import { ApiError } from '../../api/client'
import { Notice } from '../../components/States'
import { restoreSession } from '../session'
import { verifyEmail } from '../api'
import { AuthCard } from '../components/AuthCard'

/** 驗證信的連結:`/admin/verify#token=…`。token 放在 # 後面,不會送到伺服器的存取紀錄 */
function tokenFrom(hash: string): string | null {
  return new URLSearchParams(hash.replace(/^#/, '')).get('token')
}

export function VerifyEmailPage() {
  const location = useLocation()
  const [token] = useState(() => tokenFrom(location.hash))
  const mutation = useMutation({ mutationFn: () => verifyEmail(token!) })
  // 開啟頁面就送出一次(信箱的連結預覽 / 掃描程式不會執行 JavaScript,所以不會誤用掉連結)。
  // 用 ref 擋掉開發模式下 effect 重複執行:token 只能用一次,第二次會變成「已使用」
  const started = useRef(false)

  useEffect(() => {
    if (!token || started.current) return
    started.current = true
    mutation.mutate(undefined, {
      // 如果這個瀏覽器已經登入,刷新「已驗證」狀態
      onSuccess: () => void restoreSession(),
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token])

  if (!token || token.length < 16) {
    return (
      <AuthCard title="連結無效" subtitle="">
        <Notice tone="error">這個驗證連結不完整。請回到信件重新點開,或登入後重新寄送。</Notice>
        <Link to="/admin" className="btn btn-primary btn-block">
          前往後台
        </Link>
      </AuthCard>
    )
  }

  if (mutation.isSuccess) {
    return (
      <AuthCard title="Email 已驗證" subtitle="">
        <Notice tone="success">{mutation.data.message}</Notice>
        <Link to="/admin" className="btn btn-primary btn-block">
          前往後台
        </Link>
      </AuthCard>
    )
  }

  if (mutation.isError) {
    const err = mutation.error
    return (
      <AuthCard title="驗證失敗" subtitle="">
        <Notice tone="error">
          {err instanceof ApiError ? err.message : '驗證失敗,請再試一次。'}
        </Notice>
        <Link to="/admin" className="btn btn-primary btn-block">
          前往後台
        </Link>
      </AuthCard>
    )
  }

  return (
    <AuthCard title="驗證中…" subtitle="">
      <p className="muted">正在確認你的 Email。</p>
    </AuthCard>
  )
}
