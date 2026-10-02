import { useMutation } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { Link, Navigate, useLocation, useNavigate } from 'react-router-dom'
import { ApiError } from '../../api/client'
import { Notice } from '../../components/States'
import { validateEmail } from '../../lib/validate'
import { login } from '../api'
import { AuthCard, TextField } from '../components/AuthCard'
import { safeReturnPath } from '../paths'
import { setSession, useSession } from '../session'

export function LoginPage() {
  const session = useSession()
  const navigate = useNavigate()
  const location = useLocation()
  const from = safeReturnPath((location.state as { from?: string } | null)?.from)

  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [errors, setErrors] = useState<{ email?: string; password?: string }>({})

  const mutation = useMutation({
    mutationFn: () => login(email.trim(), password),
    onSuccess: (r) => {
      if (setSession(r.token, r.user)) navigate(from, { replace: true })
    },
  })

  if (session) return <Navigate to={from} replace />

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const found = {
      email: validateEmail(email) ?? undefined,
      password: password ? undefined : '請輸入密碼',
    }
    setErrors(found)
    if (!found.email && !found.password) mutation.mutate()
  }

  const err = mutation.error
  // 後端對「帳號不存在」與「密碼錯誤」回應完全相同(避免洩漏哪些 Email 已註冊),這裡也只顯示一種訊息
  const message =
    err instanceof ApiError
      ? err.status === 401
        ? 'Email 或密碼錯誤'
        : err.message
      : err
        ? '登入失敗,請再試一次。'
        : null

  return (
    <AuthCard title="登入" subtitle="登入後管理你的店家、預約與團隊。">
      {message && <Notice tone="error">{message}</Notice>}
      <form className="form form-plain" onSubmit={submit} noValidate>
        <TextField label="Email" error={errors.email}>
          <input
            name="email"
            type="email"
            autoComplete="email"
            inputMode="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            aria-invalid={!!errors.email}
          />
        </TextField>
        <TextField label="密碼" error={errors.password}>
          <input
            name="password"
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            aria-invalid={!!errors.password}
          />
        </TextField>
        <button type="submit" className="btn btn-primary btn-block" disabled={mutation.isPending}>
          {mutation.isPending ? '登入中…' : '登入'}
        </button>
      </form>
      <p className="auth-switch">
        還沒有帳號?
        <Link to="/admin/register" state={location.state}>
          註冊
        </Link>
      </p>
    </AuthCard>
  )
}
