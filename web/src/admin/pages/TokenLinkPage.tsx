import { useMutation } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import { Link, useLocation } from 'react-router-dom'
import { ApiError } from '../../api/client'
import { Notice } from '../../components/States'
import { restoreSession } from '../session'
import { AuthCard } from '../components/AuthCard'

/** 信中連結的 token:`/admin/…#token=…`。token 放在 # 後面,不會送到伺服器的存取紀錄 */
function tokenFrom(hash: string): string | null {
  return new URLSearchParams(hash.replace(/^#/, '')).get('token')
}

interface Props {
  /** 用 token 呼叫的 API(不需要登入:常在另一台裝置開信) */
  action: (token: string) => Promise<{ message: string }>
  pendingTitle: string
  pendingText: string
  successTitle: string
  failureTitle: string
  /** token 缺少或格式不對時的說明 */
  brokenText: string
}

/** 打開信中的一次性連結就自動送出(驗證 Email、確認更換 Email…) */
export function TokenLinkPage(props: Props) {
  const location = useLocation()
  const [token] = useState(() => tokenFrom(location.hash))
  const mutation = useMutation({ mutationFn: () => props.action(token!) })
  // 開啟頁面就送出一次(信箱的連結預覽 / 掃描程式不會執行 JavaScript,所以不會誤用掉連結)。
  // 用 ref 擋掉開發模式下 effect 重複執行:token 只能用一次,第二次會變成「已使用」
  const started = useRef(false)

  useEffect(() => {
    if (!token || token.length < 16 || started.current) return
    started.current = true
    mutation.mutate(undefined, {
      // 如果這個瀏覽器已經登入,刷新帳號狀態(已驗證 / 新的 Email)
      onSuccess: () => void restoreSession(),
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token])

  const home = (
    <Link to="/admin" className="btn btn-primary btn-block">
      前往後台
    </Link>
  )

  if (!token || token.length < 16) {
    return (
      <AuthCard title="連結無效" subtitle="">
        <Notice tone="error">{props.brokenText}</Notice>
        {home}
      </AuthCard>
    )
  }

  if (mutation.isSuccess) {
    return (
      <AuthCard title={props.successTitle} subtitle="">
        <Notice tone="success">{mutation.data.message}</Notice>
        {home}
      </AuthCard>
    )
  }

  if (mutation.isError) {
    const err = mutation.error
    return (
      <AuthCard title={props.failureTitle} subtitle="">
        <Notice tone="error">
          {err instanceof ApiError ? err.message : '操作失敗,請再試一次。'}
        </Notice>
        {home}
      </AuthCard>
    )
  }

  return (
    <AuthCard title={props.pendingTitle} subtitle="">
      <p className="muted">{props.pendingText}</p>
    </AuthCard>
  )
}
