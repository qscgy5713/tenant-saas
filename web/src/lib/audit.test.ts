import { describe, expect, it } from 'vitest'
import type { AuditEntry } from '../admin/types'
import { actionLabel, actorLabel, describeDetail } from './audit'

const entry = (over: Partial<AuditEntry>): AuditEntry => ({
  id: 1,
  created_at: '2026-10-05T02:00:00Z',
  actor_type: 'user',
  actor_user_id: 'u1',
  actor_name: '林美玲',
  action: 'x',
  entity_type: 'x',
  entity_id: null,
  detail: {},
  ...over,
})

describe('actionLabel', () => {
  it('已知動作翻成中文', () => {
    expect(actionLabel('booking.cancelled')).toBe('取消預約')
    expect(actionLabel('member.role_changed')).toBe('變更角色')
  })

  it('未知動作(後端新增、前端還沒翻譯)原樣顯示,不壞掉', () => {
    expect(actionLabel('invoice.created')).toBe('invoice.created')
  })
})

describe('actorLabel', () => {
  it('員工 / 顧客 / 系統', () => {
    expect(actorLabel(entry({}))).toBe('林美玲')
    expect(
      actorLabel(entry({ actor_type: 'customer', actor_user_id: null, actor_name: null })),
    ).toBe('顧客')
    expect(actorLabel(entry({ actor_type: 'system', actor_user_id: null, actor_name: null }))).toBe(
      '系統',
    )
  })

  it('成員已離開(查不到姓名)→ 不顯示 null', () => {
    expect(actorLabel(entry({ actor_name: null }))).toBe('已離開的成員')
  })
})

describe('describeDetail', () => {
  const tz = 'Asia/Taipei'

  it('角色變更', () => {
    expect(
      describeDetail(
        entry({ action: 'member.role_changed', detail: { from: 'staff', to: 'manager' } }),
        tz,
      ),
    ).toBe('員工 → 管理者')
  })

  it('服務修改:只列出有改的欄位,價格從「分」換成「元」', () => {
    const e = entry({
      action: 'service.updated',
      detail: { changed: { price_cents: 60000, active: false } },
    })
    expect(describeDetail(e, tz)).toBe('價格 → 600 元、啟用狀態 → 停用')
  })

  it('付款相關:方案名稱翻成中文;不認得的方案顯示原值', () => {
    const changed = entry({
      action: 'billing.plan_changed',
      detail: { from: 'free', to: 'pro', subscription_status: 'active' },
    })
    expect(describeDetail(changed, tz)).toBe('免費版 → 專業版(依付款狀態自動調整)')
    expect(
      describeDetail(
        entry({ action: 'billing.plan_changed', detail: { from: 'pro', to: 'legacy' } }),
        tz,
      ),
    ).toContain('legacy')
    expect(
      describeDetail(
        entry({ action: 'billing.checkout_started', detail: { plan: 'business' } }),
        tz,
      ),
    ).toBe('選擇方案:企業版')
    expect(actionLabel('billing.plan_changed')).toBe('方案變更')
    expect(
      describeDetail(
        entry({ action: 'billing.payment_failed', detail: { attempt: 2, final: false } }),
        tz,
      ),
    ).toBe('第 2 次嘗試失敗,Stripe 會再試')
    expect(
      describeDetail(
        entry({ action: 'billing.payment_failed', detail: { attempt: 4, final: true } }),
        tz,
      ),
    ).toBe('第 4 次嘗試失敗,不會再重試')
    expect(actionLabel('billing.payment_failed')).toBe('訂閱扣款失敗')
  })

  it('停用 / 重新啟用成員', () => {
    expect(actionLabel('member.deactivated')).toBe('停用成員')
    expect(actionLabel('member.reactivated')).toBe('重新啟用成員')
    expect(
      describeDetail(
        entry({ action: 'member.deactivated', detail: { role: 'staff', self: false } }),
        tz,
      ),
    ).toBe('角色:員工')
    expect(
      describeDetail(
        entry({ action: 'member.deactivated', detail: { role: 'staff', self: true } }),
        tz,
      ),
    ).toBe('本人退出')
    expect(
      describeDetail(entry({ action: 'member.reactivated', detail: { role: 'manager' } }), tz),
    ).toContain('管理者')
  })

  it('匯出與查看顧客資料:標籤與細節(不含任何個資欄位)', () => {
    expect(actionLabel('audit.exported')).toBe('匯出稽核日誌')
    expect(actionLabel('customer.viewed')).toBe('查看顧客資料')
    expect(actionLabel('customer.listed')).toBe('瀏覽顧客清單')
    expect(describeDetail(entry({ action: 'audit.exported', detail: { rows: 120 } }), tz)).toBe(
      '共 120 筆',
    )
    expect(
      describeDetail(
        entry({ action: 'customer.listed', detail: { count: 3, offset: 0, searched: true } }),
        tz,
      ),
    ).toBe('搜尋,看到 3 位顧客')
    expect(
      describeDetail(
        entry({ action: 'customer.listed', detail: { count: 50, offset: 0, searched: false } }),
        tz,
      ),
    ).toBe('瀏覽,看到 50 位顧客')
    expect(describeDetail(entry({ action: 'customer.viewed', detail: { history: 7 } }), tz)).toBe(
      '看了 7 筆預約紀錄',
    )
    for (const action of ['audit.exported', 'customer.listed', 'customer.viewed']) {
      expect(() => describeDetail(entry({ action, detail: {} }), tz)).not.toThrow()
      expect(describeDetail(entry({ action, detail: {} }), tz)).toBeNull()
    }
  })

  it('改期顯示店家時區的前後時間', () => {
    const e = entry({
      action: 'booking.rescheduled',
      detail: { from_starts_at: '2026-10-05T02:00:00Z', to_starts_at: '2026-10-06T06:30:00Z' },
    })
    const text = describeDetail(e, tz)!
    expect(text).toContain('10:00')
    expect(text).toContain('14:30')
    expect(text).toContain('→')
  })

  it('取消:系統取消顯示原因,人為取消顯示原狀態', () => {
    expect(
      describeDetail(
        entry({ action: 'booking.cancelled', detail: { reason: 'confirmation_expired' } }),
        tz,
      ),
    ).toBe('超過期限未確認')
    expect(
      describeDetail(
        entry({ action: 'booking.cancelled', detail: { from: 'pending', to: 'cancelled' } }),
        tz,
      ),
    ).toBe('原狀態:待確認')
  })

  it('detail 缺欄位或格式不符 → null,不丟例外', () => {
    for (const action of [
      'service.updated',
      'booking.rescheduled',
      'billing.plan_changed',
      'billing.payment_failed',
      'billing.checkout_started',
      'member.role_changed',
      'booking.created',
      'unknown.thing',
    ]) {
      expect(() => describeDetail(entry({ action, detail: {} }), tz)).not.toThrow()
    }
    expect(describeDetail(entry({ action: 'booking.rescheduled', detail: {} }), tz)).toBeNull()
    expect(
      describeDetail(entry({ action: 'service.updated', detail: { changed: 'oops' } }), tz),
    ).toBeNull()
  })
})
