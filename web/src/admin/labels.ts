import type { BookingStatus } from '../api/types'

export const STATUS_LABEL: Record<BookingStatus, string> = {
  pending: '待確認',
  confirmed: '已確認',
  cancelled: '已取消',
  completed: '已完成',
  no_show: '未到',
}
