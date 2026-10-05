import { ApiError } from '../../api/client'
import { formatDateTime } from '../../lib/time'
import { useRescheduleBooking } from '../queries'
import { useShop } from '../ShopContext'
import type { AdminBooking } from '../types'
import { ConfirmDialog } from './Modal'

export interface Move {
  booking: AdminBooking
  /** 新的開始時間(ISO) */
  start: string
}

/**
 * 拖曳改期的確認。放開滑鼠就直接改期太容易手滑,而且會寄信給顧客,所以先問一次。
 * 能不能改(員工是否有空、營業時間、方案額度…)由後端判斷,不符就把原因顯示在這裡。
 * 父層用 key 讓每次拖曳都是全新的狀態(不會殘留上一次的錯誤)。
 */
export function MoveConfirm({
  move,
  onClose,
  onDone,
}: {
  move: Move | null
  onClose: () => void
  onDone: (message: string) => void
}) {
  const { slug, shop } = useShop()
  const mutation = useRescheduleBooking(slug)
  const tz = shop.timezone
  const error = mutation.error
  const message = error instanceof ApiError ? error.message : error ? '改期失敗,請再試一次。' : null

  return (
    <ConfirmDialog
      open={move !== null}
      title="改期?"
      message={
        move
          ? `${move.booking.customer_name} 的「${move.booking.service_name}」從 ${formatDateTime(move.booking.starts_at, tz)} 改到 ${formatDateTime(move.start, tz)}。服務人員與服務不變,系統會寄通知信給顧客。`
          : ''
      }
      confirmLabel="確定改期"
      pending={mutation.isPending}
      error={message}
      onConfirm={() =>
        move &&
        mutation.mutate(
          { id: move.booking.id, start: move.start },
          {
            onSuccess: () => {
              onDone('已改期,並寄出通知信給顧客。')
              onClose()
            },
          },
        )
      }
      onClose={onClose}
    />
  )
}
