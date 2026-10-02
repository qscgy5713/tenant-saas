import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { ApiError } from '../../api/client'
import type { SlotSource } from '../../api/queries'
import { SlotPicker } from '../../components/SlotPicker'
import { Notice } from '../../components/States'
import type { TimeOption } from '../../lib/slots'
import { formatDateTime } from '../../lib/time'
import { getBookingAvailability, rescheduleBooking } from '../api'
import { bookingsKey } from '../queries'
import { useShop } from '../ShopContext'
import type { AdminBooking } from '../types'
import { Modal } from './Modal'

/** 員工替顧客改期。同一位員工與服務;顧客會收到通知信。 */
export function RescheduleModal({
  booking,
  open,
  onClose,
}: {
  booking: AdminBooking
  open: boolean
  onClose: () => void
}) {
  return (
    <Modal open={open} title="改期" onClose={onClose} size="lg">
      <RescheduleForm booking={booking} onClose={onClose} />
    </Modal>
  )
}

function RescheduleForm({ booking, onClose }: { booking: AdminBooking; onClose: () => void }) {
  const { slug, shop } = useShop()
  const queryClient = useQueryClient()
  const [choice, setChoice] = useState<TimeOption | null>(null)
  const tz = shop.timezone

  // 時段來源:排除這筆預約自己,所以自己原本的時段、以及與它重疊的時段都選得到
  const source: SlotSource = {
    key: ['availability', 'admin-booking', slug, booking.id],
    fetch: (from, to) => getBookingAvailability(slug, booking.id, from, to),
  }

  const mutation = useMutation({
    mutationFn: () => rescheduleBooking(slug, booking.id, choice!.start),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: bookingsKey(slug) })
      queryClient.invalidateQueries({ queryKey: ['availability'] })
      queryClient.invalidateQueries({ queryKey: ['admin', 'plan', slug] })
      onClose()
    },
  })

  const error = mutation.error
  const message = error instanceof ApiError ? error.message : error ? '改期失敗,請再試一次。' : null

  return (
    <>
      <p className="muted small">
        {booking.customer_name} · {booking.service_name} · {booking.staff_name}
        <br />
        目前:{formatDateTime(booking.starts_at, tz)}
        。服務人員與服務不變,改期後系統會寄通知信給顧客。
      </p>
      <SlotPicker
        source={source}
        timezone={tz}
        selectedStart={choice?.start ?? null}
        onSelect={setChoice}
      />
      {message && <Notice tone="error">{message}</Notice>}
      <div className="actions">
        <button
          type="button"
          className="btn btn-primary"
          disabled={!choice || mutation.isPending}
          onClick={() => mutation.mutate()}
        >
          {mutation.isPending
            ? '改期中…'
            : choice
              ? `改到 ${formatDateTime(choice.start, tz)}`
              : '請先選擇新時間'}
        </button>
        <button type="button" className="btn btn-secondary" onClick={onClose}>
          取消
        </button>
      </div>
    </>
  )
}
