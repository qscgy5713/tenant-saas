import type { ReactNode } from 'react'
import { ApiError } from '../api/client'

export function Loading({ label = '載入中…' }: { label?: string }) {
  return (
    <div className="state" role="status" aria-live="polite">
      <span className="spinner" aria-hidden="true" />
      <span>{label}</span>
    </div>
  )
}

export function ErrorState({
  error,
  onRetry,
  title = '發生問題',
  children,
}: {
  error: unknown
  onRetry?: () => void
  title?: string
  children?: ReactNode
}) {
  const message = error instanceof ApiError ? error.message : '發生未預期的錯誤,請重新整理頁面。'
  const requestId = error instanceof ApiError ? error.requestId : undefined
  // 4xx 重試也一樣,不提供「重試」;網路 / 伺服器問題才提供
  const retryable = !(error instanceof ApiError && error.isClientError)
  return (
    <div className="state state-error" role="alert">
      <strong>{title}</strong>
      <p>{message}</p>
      {requestId && <p className="muted small">錯誤代碼:{requestId}</p>}
      {children}
      {onRetry && retryable && (
        <button type="button" className="btn btn-secondary" onClick={onRetry}>
          重試
        </button>
      )}
    </div>
  )
}

export function Notice({
  tone,
  children,
}: {
  tone: 'info' | 'success' | 'error' | 'warning'
  children: ReactNode
}) {
  // 錯誤用 alert 讓螢幕閱讀器立刻唸出;其餘用 status
  return (
    <div className={`notice notice-${tone}`} role={tone === 'error' ? 'alert' : 'status'}>
      {children}
    </div>
  )
}
