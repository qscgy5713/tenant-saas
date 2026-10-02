import { useMutation } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { Link } from 'react-router-dom'
import { ApiError } from '../../api/client'
import { Notice } from '../../components/States'
import { validateEmail } from '../../lib/validate'
import { forgotPassword } from '../api'
import { AuthCard, TextField } from '../components/AuthCard'

export function ForgotPasswordPage() {
  const [email, setEmail] = useState('')
  const [error, setError] = useState<string>()
  const mutation = useMutation({ mutationFn: () => forgotPassword(email.trim()) })

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const found = validateEmail(email) ?? undefined
    setError(found)
    if (!found) mutation.mutate()
  }

  // 後端不論 Email 是否註冊都回同一個訊息,這裡直接顯示它
  if (mutation.isSuccess) {
    return (
      <AuthCard title="請查看信箱" subtitle="">
        <Notice tone="success">{mutation.data.message}</Notice>
        <p className="auth-switch">
          <Link to="/admin/login">回到登入</Link>
        </p>
      </AuthCard>
    )
  }

  const err = mutation.error
  const message = err instanceof ApiError ? err.message : err ? '送出失敗,請再試一次。' : null

  return (
    <AuthCard title="忘記密碼" subtitle="輸入註冊時使用的 Email,我們會寄一封重設密碼的信給你。">
      {message && <Notice tone="error">{message}</Notice>}
      <form className="form form-plain" onSubmit={submit} noValidate>
        <TextField label="Email" error={error}>
          <input
            name="email"
            type="email"
            autoComplete="email"
            inputMode="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            aria-invalid={!!error}
          />
        </TextField>
        <button type="submit" className="btn btn-primary btn-block" disabled={mutation.isPending}>
          {mutation.isPending ? '送出中…' : '寄送重設信'}
        </button>
      </form>
      <p className="auth-switch">
        <Link to="/admin/login">回到登入</Link>
      </p>
    </AuthCard>
  )
}
