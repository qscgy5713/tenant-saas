import { useMutation } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { Link, useLocation } from 'react-router-dom'
import { ApiError } from '../../api/client'
import { Notice } from '../../components/States'
import { validatePassword } from '../../lib/validate'
import { resetPassword } from '../api'
import { AuthCard, TextField } from '../components/AuthCard'

/** 重設信的連結:`/admin/reset#token=…`。token 放在 # 後面,不會送到伺服器 */
function tokenFrom(hash: string): string | null {
  return new URLSearchParams(hash.replace(/^#/, '')).get('token')
}

export function ResetPasswordPage() {
  const location = useLocation()
  const [token] = useState(() => tokenFrom(location.hash))
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [errors, setErrors] = useState<{ password?: string; confirm?: string }>({})
  const mutation = useMutation({ mutationFn: () => resetPassword(token!, password) })

  if (!token || token.length < 16) {
    return (
      <AuthCard title="連結無效" subtitle="">
        <Notice tone="error">這個重設連結不完整。請回到信件重新點開,或重新申請。</Notice>
        <p className="auth-switch">
          <Link to="/admin/forgot">重新申請</Link>
        </p>
      </AuthCard>
    )
  }

  if (mutation.isSuccess) {
    return (
      <AuthCard title="密碼已更新" subtitle="">
        <Notice tone="success">{mutation.data.message}</Notice>
        <Link to="/admin/login" className="btn btn-primary btn-block">
          前往登入
        </Link>
      </AuthCard>
    )
  }

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const found = {
      password: validatePassword(password) ?? undefined,
      confirm: confirm === password ? undefined : '兩次輸入的密碼不一致',
    }
    setErrors(found)
    if (!found.password && !found.confirm) mutation.mutate()
  }

  const err = mutation.error
  const expired = err instanceof ApiError && err.status === 400 && /連結/.test(err.message)
  const message = err instanceof ApiError ? err.message : err ? '重設失敗,請再試一次。' : null

  return (
    <AuthCard title="設定新密碼" subtitle="設定後,其他裝置上的登入都會被登出。">
      {message && <Notice tone="error">{message}</Notice>}
      {expired && (
        <p className="auth-switch">
          <Link to="/admin/forgot">重新申請</Link>
        </p>
      )}
      <form className="form form-plain" onSubmit={submit} noValidate>
        <TextField label="新密碼" error={errors.password}>
          <input
            name="password"
            type="password"
            autoComplete="new-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            aria-invalid={!!errors.password}
          />
        </TextField>
        <TextField label="再輸入一次" error={errors.confirm}>
          <input
            name="confirm"
            type="password"
            autoComplete="new-password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            aria-invalid={!!errors.confirm}
          />
        </TextField>
        <button type="submit" className="btn btn-primary btn-block" disabled={mutation.isPending}>
          {mutation.isPending ? '更新中…' : '更新密碼'}
        </button>
      </form>
    </AuthCard>
  )
}
