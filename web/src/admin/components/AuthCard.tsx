import type { ReactNode } from 'react'
import { CalendarIcon } from '../../components/Icons'
import { useTitle } from '../../lib/useTitle'

/** 登入 / 註冊 / 接受邀請共用的置中卡片 */
export function AuthCard({
  title,
  subtitle,
  children,
}: {
  title: string
  subtitle?: string
  children: ReactNode
}) {
  useTitle(`${title} · 店家後台`)
  return (
    <div className="auth-page">
      <div className="auth-card">
        <div className="auth-brand">
          <span className="brand-mark">
            <CalendarIcon width={20} height={20} />
          </span>
          <span>店家後台</span>
        </div>
        <h1>{title}</h1>
        {subtitle && <p className="muted">{subtitle}</p>}
        {children}
      </div>
    </div>
  )
}

/** 帶標籤與錯誤訊息的輸入欄位。用 label 包住 input,點標籤就聚焦 */
export function TextField({
  label,
  error,
  hint,
  children,
}: {
  label: string
  error?: string | null
  hint?: string
  children: React.ReactElement<{ 'aria-invalid'?: boolean }>
}) {
  return (
    <div className={`field ${error ? 'field-error' : ''}`}>
      <label>
        <span className="field-label">{label}</span>
        {children}
      </label>
      {hint && !error && <p className="field-hint">{hint}</p>}
      {error && <p className="field-msg">{error}</p>}
    </div>
  )
}
