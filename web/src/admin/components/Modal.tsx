import { useEffect, useId, useRef, type ReactNode } from 'react'
import { XIcon } from '../../components/Icons'

/**
 * 原生 <dialog>:瀏覽器負責焦點鎖定、Esc 關閉、背景不可操作,也有正確的無障礙語意。
 * 只在開啟時才渲染內容,所以每次開啟表單都是乾淨的初始狀態。
 */
export function Modal({
  open,
  title,
  onClose,
  children,
  size = 'md',
}: {
  open: boolean
  title: string
  onClose: () => void
  children: ReactNode
  size?: 'md' | 'lg'
}) {
  const ref = useRef<HTMLDialogElement>(null)
  const titleId = useId()

  useEffect(() => {
    const dialog = ref.current
    if (!dialog) return
    if (open && !dialog.open) dialog.showModal()
    if (!open && dialog.open) dialog.close()
  }, [open])

  return (
    <dialog
      ref={ref}
      className={`modal modal-${size}`}
      aria-labelledby={titleId}
      onClose={onClose}
      onClick={(e) => {
        // 點背景(dialog 本身,不是內容)關閉
        if (e.target === ref.current) onClose()
      }}
    >
      {open && (
        <div className="modal-panel">
          <header className="modal-head">
            <h2 id={titleId}>{title}</h2>
            <button
              type="button"
              className="icon-btn icon-btn-sm"
              aria-label="關閉"
              onClick={onClose}
            >
              <XIcon width={18} height={18} />
            </button>
          </header>
          <div className="modal-content">{children}</div>
        </div>
      )}
    </dialog>
  )
}

export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  danger = false,
  pending = false,
  error,
  onConfirm,
  onClose,
}: {
  open: boolean
  title: string
  message: ReactNode
  confirmLabel: string
  danger?: boolean
  pending?: boolean
  error?: string | null
  onConfirm: () => void
  onClose: () => void
}) {
  return (
    <Modal open={open} title={title} onClose={onClose}>
      <p>{message}</p>
      {error && (
        <div className="notice notice-error" role="alert">
          {error}
        </div>
      )}
      <div className="actions">
        <button
          type="button"
          className={`btn ${danger ? 'btn-danger' : 'btn-primary'}`}
          onClick={onConfirm}
          disabled={pending}
        >
          {pending ? '處理中…' : confirmLabel}
        </button>
        <button type="button" className="btn btn-secondary" onClick={onClose}>
          取消
        </button>
      </div>
    </Modal>
  )
}
