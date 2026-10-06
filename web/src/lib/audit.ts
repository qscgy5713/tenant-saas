import type { AuditEntry } from '../admin/types'
import { formatDateTime } from './time'

const PLAN_NAME: Record<string, string> = { free: '免費版', pro: '專業版', business: '企業版' }

const ACTION_LABEL: Record<string, string> = {
  'tenant.created': '建立店家',
  'service.created': '新增服務',
  'service.updated': '修改服務',
  'service.deleted': '刪除服務',
  'member.services_replaced': '更新可提供的服務',
  'member.working_hours_replaced': '更新營業時間',
  'member.role_changed': '變更角色',
  'member.removed': '移除成員',
  'member.left': '退出團隊',
  'member.deactivated': '停用成員',
  'booking.reassigned': '改派預約',
  'member.reactivated': '重新啟用成員',
  'time_off.created': '新增休假',
  'time_off.deleted': '刪除休假',
  'invitation.created': '發出邀請',
  'invitation.revoked': '撤銷邀請',
  'invitation.accepted': '接受邀請',
  'booking.created': '建立預約(店家代客)',
  'booking.requested': '顧客提出預約申請',
  'booking.confirmed': '顧客確認預約',
  'booking.rescheduled': '預約改期',
  'booking.cancelled': '取消預約',
  'booking.completed': '標記為已完成',
  'booking.no_show': '標記為未到',
  'booking.notes_updated': '更新預約備註',
  'billing.checkout_started': '開始線上訂閱',
  'billing.plan_changed': '方案變更',
  'audit.exported': '匯出稽核日誌',
  'customer.listed': '瀏覽顧客清單',
  'customer.viewed': '查看顧客資料',
  'billing.payment_failed': '訂閱扣款失敗',
  'billing.payment_recovered': '訂閱付款已恢復',
  'customer.anonymized': '匿名化顧客資料',
  'tenant.retention_changed': '調整顧客資料保留天數',
  'tenant.deletion_requested': '申請刪除店家',
  'tenant.deletion_cancelled': '取消刪除店家',
  'audit.purged': '清除過期的稽核紀錄',
}

const ROLE: Record<string, string> = { owner: '擁有者', manager: '管理者', staff: '員工' }
const STATUS: Record<string, string> = {
  pending: '待確認',
  confirmed: '已確認',
  cancelled: '已取消',
  completed: '已完成',
  no_show: '未到',
}
const CANCEL_REASON: Record<string, string> = {
  monthly_limit: '本月預約已額滿',
  no_longer_bookable: '確認時該時段已無法預約',
  slot_taken: '時段已被搶先預約',
  confirmation_expired: '超過期限未確認',
}
const FIELD: Record<string, string> = {
  name: '名稱',
  duration_minutes: '時長',
  price_cents: '價格',
  active: '啟用狀態',
}

/** 未知的動作(後端新增、前端還沒翻譯)原樣顯示,不要壞掉 */
export function actionLabel(action: string): string {
  return ACTION_LABEL[action] ?? action
}

export function actorLabel(entry: AuditEntry): string {
  if (entry.actor_type === 'customer') return '顧客'
  if (entry.actor_type === 'system') return '系統'
  return entry.actor_name ?? '已離開的成員'
}

const str = (v: unknown): string | null => (typeof v === 'string' ? v : null)
const num = (v: unknown): number | null => (typeof v === 'number' ? v : null)

/** 一行白話的細節說明,沒有可說的就回傳 null。detail 的格式見 src/audit.rs 與各 handler,刻意不含個資。 */
export function describeDetail(entry: AuditEntry, timeZone: string): string | null {
  const d = entry.detail
  switch (entry.action) {
    case 'tenant.created':
      return str(d.slug) ? `預約頁代稱:${str(d.slug)}` : null
    case 'service.created':
      return num(d.duration_minutes) !== null
        ? `${num(d.duration_minutes)} 分鐘、${(num(d.price_cents) ?? 0) / 100} 元`
        : null
    case 'service.updated': {
      const changed = d.changed
      if (!changed || typeof changed !== 'object') return null
      const parts = Object.entries(changed as Record<string, unknown>).map(([k, v]) => {
        const label = FIELD[k] ?? k
        if (k === 'price_cents') return `${label} → ${(num(v) ?? 0) / 100} 元`
        if (k === 'active') return `${label} → ${v ? '啟用' : '停用'}`
        if (k === 'duration_minutes') return `${label} → ${v} 分鐘`
        return `${label} → ${String(v)}`
      })
      return parts.length ? parts.join('、') : null
    }
    case 'customer.anonymized':
      return d.reason === 'retention' ? '超過保留天數,系統自動匿名化' : null
    case 'tenant.retention_changed':
      return num(d.from) !== null && num(d.to) !== null
        ? `${num(d.from)} 天 → ${num(d.to)} 天`
        : null
    case 'tenant.deletion_requested':
      return str(d.scheduled_at)
        ? `預定 ${formatDateTime(str(d.scheduled_at)!, timeZone)} 永久刪除`
        : null
    case 'audit.purged':
      return num(d.rows) !== null ? `清除 ${num(d.rows)} 筆超過保留期限的紀錄` : null
    case 'member.role_changed':
      return `${ROLE[str(d.from) ?? ''] ?? d.from} → ${ROLE[str(d.to) ?? ''] ?? d.to}`
    case 'member.deactivated':
      return d.self === true
        ? '本人退出'
        : str(d.role)
          ? `角色:${ROLE[str(d.role)!] ?? d.role}`
          : null
    case 'member.reactivated':
    case 'member.removed':
      return str(d.role) ? `角色:${ROLE[str(d.role)!] ?? d.role}` : null
    case 'invitation.created':
    case 'invitation.revoked':
    case 'invitation.accepted':
      return str(d.role) ? `角色:${ROLE[str(d.role)!] ?? d.role}` : null
    case 'member.services_replaced':
      return num(d.count) !== null ? `共 ${num(d.count)} 項服務` : null
    case 'member.working_hours_replaced':
      return num(d.entries) !== null ? `共 ${num(d.entries)} 個時段` : null
    case 'booking.created':
    case 'booking.requested':
      return str(d.starts_at) ? `預約時間:${formatDateTime(str(d.starts_at)!, timeZone)}` : null
    case 'billing.checkout_started':
      return str(d.plan) ? `選擇方案:${PLAN_NAME[str(d.plan)!] ?? str(d.plan)}` : null
    case 'audit.exported':
      return num(d.rows) !== null ? `共 ${num(d.rows)} 筆` : null
    case 'customer.listed':
      return num(d.count) !== null
        ? `${d.searched === true ? '搜尋' : '瀏覽'},看到 ${num(d.count)} 位顧客`
        : null
    case 'customer.viewed':
      return num(d.history) !== null ? `看了 ${num(d.history)} 筆預約紀錄` : null
    case 'billing.payment_failed':
      return num(d.attempt) === null
        ? null
        : d.final === true
          ? `第 ${num(d.attempt)} 次嘗試失敗,不會再重試`
          : `第 ${num(d.attempt)} 次嘗試失敗,Stripe 會再試`
    case 'billing.plan_changed': {
      const from = str(d.from)
      const to = str(d.to)
      return from && to
        ? `${PLAN_NAME[from] ?? from} → ${PLAN_NAME[to] ?? to}(依付款狀態自動調整)`
        : null
    }
    case 'booking.rescheduled':
      return str(d.from_starts_at) && str(d.to_starts_at)
        ? `${formatDateTime(str(d.from_starts_at)!, timeZone)} → ${formatDateTime(str(d.to_starts_at)!, timeZone)}`
        : null
    case 'booking.cancelled': {
      const reason = str(d.reason)
      const from = str(d.from)
      if (reason) return CANCEL_REASON[reason] ?? reason
      return from ? `原狀態:${STATUS[from] ?? from}` : null
    }
    case 'booking.completed':
    case 'booking.no_show':
      return null
    default:
      return null
  }
}

/** 篩選用的動作前綴 */
export const AUDIT_FILTERS: { value: string; label: string }[] = [
  { value: '', label: '全部' },
  { value: 'booking.', label: '預約' },
  { value: 'service.', label: '服務' },
  { value: 'member.', label: '成員' },
  { value: 'invitation.', label: '邀請' },
  { value: 'time_off.', label: '休假' },
  { value: 'billing.', label: '付款' },
  { value: 'customer.', label: '顧客資料' },
  { value: 'audit.', label: '匯出' },
]
