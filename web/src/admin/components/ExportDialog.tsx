import { useMutation } from '@tanstack/react-query'
import { useState, type FormEvent, type ReactNode } from 'react'
import { ApiError } from '../../api/client'
import { Notice } from '../../components/States'
import { saveFile } from '../../lib/download'
import { addDays, startOfDay } from '../../lib/time'
import { useToday } from '../../lib/useToday'
import { useShop } from '../ShopContext'
import { Modal } from './Modal'

export interface ExportRange {
  /** ISO 時間,含 */
  from: string
  /** ISO 時間,不含 */
  to: string
}

/**
 * 匯出 CSV 的視窗:選日期範圍(店家時區的整天,含起訖兩天)→ 呼叫 `run` → 存檔。
 * 用 `fetch` 下載而不是 `<a href>`:失敗時(例如超過筆數上限)才能把原因顯示在畫面上,
 * 而不是下載到一個內容是錯誤訊息的檔案。
 */
export function ExportDialog({
  open,
  onClose,
  title,
  description,
  successNote,
  run,
  onExported,
}: {
  open: boolean
  onClose: () => void
  title: string
  description: ReactNode
  successNote: string
  run: (range: ExportRange) => Promise<{ blob: Blob; filename: string }>
  onExported?: () => void
}) {
  return (
    <Modal open={open} title={title} onClose={onClose}>
      <ExportForm
        onClose={onClose}
        description={description}
        successNote={successNote}
        run={run}
        onExported={onExported}
      />
    </Modal>
  )
}

function ExportForm({
  onClose,
  description,
  successNote,
  run,
  onExported,
}: {
  onClose: () => void
  description: ReactNode
  successNote: string
  run: (range: ExportRange) => Promise<{ blob: Blob; filename: string }>
  onExported?: () => void
}) {
  const { shop } = useShop()
  const { today } = useToday(shop.timezone)
  const [from, setFrom] = useState(() => addDays(today, -29))
  const [to, setTo] = useState(today)
  const [problem, setProblem] = useState<string | null>(null)

  const mutation = useMutation({
    mutationFn: () =>
      run({
        from: startOfDay(from, shop.timezone).toISOString(),
        // 結束日「當天」也要算進去:到隔天 00:00 為止(不含)
        to: startOfDay(addDays(to, 1), shop.timezone).toISOString(),
      }),
    onSuccess: ({ blob, filename }) => {
      saveFile(blob, filename)
      onExported?.()
    },
  })

  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (!from || !to) return setProblem('請選擇日期範圍')
    if (to < from) return setProblem('結束日期不能早於開始日期')
    setProblem(null)
    mutation.mutate()
  }

  const shown =
    problem ??
    (mutation.error instanceof ApiError
      ? mutation.error.message
      : mutation.error
        ? '匯出失敗,請再試一次。'
        : null)

  if (mutation.isSuccess) {
    return (
      <>
        <Notice tone="success">{successNote}</Notice>
        <div className="actions">
          <button type="button" className="btn btn-primary" onClick={onClose}>
            完成
          </button>
        </div>
      </>
    )
  }

  return (
    <form className="form-plain new-booking" onSubmit={submit} noValidate>
      {shown && <Notice tone="error">{shown}</Notice>}
      <p className="muted small">{description}</p>
      <div className="field-row">
        <div className="field">
          <label>
            <span className="field-label">開始日期</span>
            <input type="date" value={from} onChange={(e) => setFrom(e.target.value)} />
          </label>
        </div>
        <div className="field">
          <label>
            <span className="field-label">結束日期</span>
            <input type="date" value={to} onChange={(e) => setTo(e.target.value)} />
          </label>
        </div>
      </div>
      <div className="actions">
        <button type="submit" className="btn btn-primary" disabled={mutation.isPending}>
          {mutation.isPending ? '匯出中…' : '匯出'}
        </button>
        <button type="button" className="btn btn-secondary" onClick={onClose}>
          取消
        </button>
      </div>
    </form>
  )
}
