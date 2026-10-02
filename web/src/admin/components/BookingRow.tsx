import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { ApiError } from '../../api/client'
import { Notice } from '../../components/States'
import { formatDayLong, formatTime, ymdInTz } from '../../lib/time'
import { useNow } from '../../lib/useNow'
import { updateBooking } from '../api'
import { STATUS_LABEL } from '../labels'
import { useShop } from '../ShopContext'
import type { AdminBooking } from '../types'
import { bookingsKey } from '../queries'
import { ConfirmDialog, Modal } from './Modal'

/** 一筆預約:時間、顧客、服務、狀態,以及可做的操作 */
export function BookingRow({
  booking,
  showDate = false,
}: {
  booking: AdminBooking
  showDate?: boolean
}) {
  const { slug, shop } = useShop()
  const queryClient = useQueryClient()
  const [asking, setAsking] = useState<'cancel' | null>(null)
  const [editingNotes, setEditingNotes] = useState(false)

  const tz = shop.timezone
  const now = useNow()
  const started = new Date(booking.starts_at).getTime() <= now
  const live = booking.status === 'pending' || booking.status === 'confirmed'

  const mutation = useMutation({
    mutationFn: (body: Parameters<typeof updateBooking>[2]) =>
      updateBooking(slug, booking.id, body),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: bookingsKey(slug) })
      queryClient.invalidateQueries({ queryKey: ['admin', 'plan', slug] })
      setAsking(null)
      setEditingNotes(false)
    },
  })
  const error =
    mutation.error instanceof ApiError
      ? mutation.error.message
      : mutation.error
        ? '操作失敗,請再試一次。'
        : null

  return (
    <li className={`row booking-row booking-${booking.status}`}>
      <div className="time-col">
        {showDate && <div className="row-sub">{formatDayLong(ymdInTz(booking.starts_at, tz))}</div>}
        {formatTime(booking.starts_at, tz)} – {formatTime(booking.ends_at, tz)}
      </div>

      <div className="row-main">
        <div className="row-title">
          {booking.customer_name}
          <span className={`badge badge-${booking.status} badge-inline`}>
            {STATUS_LABEL[booking.status]}
          </span>
        </div>
        <div className="row-sub">
          {booking.service_name} · {booking.staff_name}
        </div>
        <div className="row-sub contact">
          <a href={`mailto:${booking.customer_email}`}>{booking.customer_email}</a>
          {booking.customer_phone && (
            <a href={`tel:${booking.customer_phone}`}>{booking.customer_phone}</a>
          )}
        </div>
        {booking.notes && <div className="row-note">備註:{booking.notes}</div>}
        {error && !asking && !editingNotes && <Notice tone="error">{error}</Notice>}
      </div>

      <div className="row-actions">
        {booking.status === 'confirmed' && started && (
          <>
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              disabled={mutation.isPending}
              onClick={() => mutation.mutate({ status: 'completed' })}
            >
              完成
            </button>
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              disabled={mutation.isPending}
              onClick={() => mutation.mutate({ status: 'no_show' })}
            >
              未到
            </button>
          </>
        )}
        {live && (
          <button
            type="button"
            className="btn btn-danger-outline btn-sm"
            onClick={() => setAsking('cancel')}
          >
            取消
          </button>
        )}
        <button
          type="button"
          className="btn btn-secondary btn-sm"
          onClick={() => setEditingNotes(true)}
        >
          備註
        </button>
      </div>

      <ConfirmDialog
        open={asking === 'cancel'}
        title="取消這筆預約?"
        message={`${booking.customer_name} 的「${booking.service_name}」(${formatDayLong(ymdInTz(booking.starts_at, tz))} ${formatTime(booking.starts_at, tz)})會被取消,時段會釋出。取消後無法復原。`}
        confirmLabel="確定取消"
        danger
        pending={mutation.isPending}
        error={asking ? error : null}
        onConfirm={() => mutation.mutate({ status: 'cancelled' })}
        onClose={() => {
          setAsking(null)
          mutation.reset()
        }}
      />
      <NotesModal
        open={editingNotes}
        initial={booking.notes ?? ''}
        pending={mutation.isPending}
        error={editingNotes ? error : null}
        onSave={(notes) => mutation.mutate({ notes })}
        onClose={() => {
          setEditingNotes(false)
          mutation.reset()
        }}
      />
    </li>
  )
}

function NotesModal({
  open,
  initial,
  pending,
  error,
  onSave,
  onClose,
}: {
  open: boolean
  initial: string
  pending: boolean
  error: string | null
  onSave: (notes: string) => void
  onClose: () => void
}) {
  return (
    <Modal open={open} title="預約備註" onClose={onClose}>
      {/* 內容只在開啟時才渲染,所以輸入框的狀態每次開啟都從最新的備註開始 */}
      <NotesForm
        initial={initial}
        pending={pending}
        error={error}
        onSave={onSave}
        onClose={onClose}
      />
    </Modal>
  )
}

function NotesForm({
  initial,
  pending,
  error,
  onSave,
  onClose,
}: {
  initial: string
  pending: boolean
  error: string | null
  onSave: (notes: string) => void
  onClose: () => void
}) {
  const [value, setValue] = useState(initial)
  return (
    <>
      <div className="field">
        <label>
          <span className="field-label">備註(最多 500 字)</span>
          <textarea value={value} onChange={(e) => setValue(e.target.value)} maxLength={500} />
        </label>
        <p className="field-hint">
          備註只有店家看得到。稽核日誌只會記錄「備註有異動」,不會記錄內容。
        </p>
      </div>
      {error && <Notice tone="error">{error}</Notice>}
      <div className="actions">
        <button
          type="button"
          className="btn btn-primary"
          disabled={pending}
          onClick={() => onSave(value)}
        >
          {pending ? '儲存中…' : '儲存'}
        </button>
        <button type="button" className="btn btn-secondary" onClick={onClose}>
          取消
        </button>
      </div>
    </>
  )
}
