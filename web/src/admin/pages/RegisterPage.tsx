import { useMutation } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { Link, Navigate, useLocation, useNavigate } from 'react-router-dom'
import { ApiError } from '../../api/client'
import { Notice } from '../../components/States'
import { validateEmail, validateName, validatePassword } from '../../lib/validate'
import { register } from '../api'
import { AuthCard, TextField } from '../components/AuthCard'
import { safeReturnPath } from '../paths'
import { setSession, useSession } from '../session'

export function RegisterPage() {
  const session = useSession()
  const navigate = useNavigate()
  const location = useLocation()
  const from = safeReturnPath((location.state as { from?: string } | null)?.from)

  const [values, setValues] = useState({ name: '', email: '', password: '' })
  const [errors, setErrors] = useState<Partial<Record<keyof typeof values, string>>>({})
  const set = (key: keyof typeof values) => (e: React.ChangeEvent<HTMLInputElement>) => {
    setValues((v) => ({ ...v, [key]: e.target.value }))
    if (errors[key]) setErrors((prev) => ({ ...prev, [key]: undefined }))
  }

  const mutation = useMutation({
    mutationFn: () => register(values.name.trim(), values.email.trim(), values.password),
    onSuccess: (r) => {
      if (setSession(r.token, r.user)) navigate(from, { replace: true })
    },
  })

  if (session) return <Navigate to={from} replace />

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const found = {
      name: validateName(values.name, '姓名') ?? undefined,
      email: validateEmail(values.email) ?? undefined,
      password: validatePassword(values.password) ?? undefined,
    }
    setErrors(found)
    if (!found.name && !found.email && !found.password) mutation.mutate()
  }

  const message = mutation.error
    ? mutation.error instanceof ApiError
      ? mutation.error.message
      : '註冊失敗,請再試一次。'
    : null

  return (
    <AuthCard title="建立帳號" subtitle="建立帳號後,就能開店或接受邀請加入團隊。">
      {message && <Notice tone="error">{message}</Notice>}
      <form className="form form-plain" onSubmit={submit} noValidate>
        <TextField label="姓名" error={errors.name}>
          <input
            name="name"
            autoComplete="name"
            value={values.name}
            onChange={set('name')}
            aria-invalid={!!errors.name}
          />
        </TextField>
        <TextField
          label="Email"
          error={errors.email}
          hint="被邀請加入團隊時,請使用收到邀請信的 Email"
        >
          <input
            name="email"
            type="email"
            autoComplete="email"
            inputMode="email"
            value={values.email}
            onChange={set('email')}
            aria-invalid={!!errors.email}
          />
        </TextField>
        <TextField label="密碼" error={errors.password} hint="至少 8 個字">
          <input
            name="password"
            type="password"
            autoComplete="new-password"
            value={values.password}
            onChange={set('password')}
            aria-invalid={!!errors.password}
          />
        </TextField>
        <button type="submit" className="btn btn-primary btn-block" disabled={mutation.isPending}>
          {mutation.isPending ? '建立中…' : '建立帳號'}
        </button>
      </form>
      <p className="auth-switch">
        已經有帳號?
        <Link to="/admin/login" state={location.state}>
          登入
        </Link>
      </p>
    </AuthCard>
  )
}
