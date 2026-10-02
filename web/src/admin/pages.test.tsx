import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { HttpResponse, http } from 'msw'
import { beforeAll, beforeEach, describe, expect, it } from 'vitest'
import { freezeNow } from '../test/freeze'
import { renderApp } from '../test/render'
import { API, error, server, taipei } from '../test/server'
import { SHOP, USER, loginAs, mockShopMe, preloadAdmin, spy } from '../test/admin'
import { canEditSchedule, canRemove } from './permissions'
import type { AdminBooking, AdminService, Member } from './types'

beforeAll(preloadAdmin)
beforeEach(() => {
  freezeNow()
  loginAs()
})

const booking = (over: Partial<AdminBooking> = {}): AdminBooking => ({
  id: 'b1',
  status: 'confirmed',
  starts_at: taipei('2026-10-05', '14:00'),
  ends_at: taipei('2026-10-05', '15:00'),
  notes: null,
  service_id: 's1',
  service_name: '剪髮',
  staff_id: 'u-owner',
  staff_name: '林美玲',
  customer_name: '王小明',
  customer_email: 'ming@customer.example.com',
  customer_phone: '0912-345-678',
  ...over,
})

const service = (over: Partial<AdminService> = {}): AdminService => ({
  id: 's1',
  name: '剪髮',
  duration_minutes: 60,
  price_cents: 50000,
  active: true,
  ...over,
})

const member = (over: Partial<Member>): Member => ({
  user_id: 'u1',
  name: '成員',
  email: 'm@example.com',
  role: 'staff',
  ...over,
})

function mockBookings(items: AdminBooking[]) {
  server.use(
    http.get(`${API}/t/demo-salon/bookings`, () =>
      HttpResponse.json({ items, limit: 100, offset: 0 }),
    ),
  )
}

describe('權限:canRemove 與後端 Role::can_manage 一致', () => {
  const me = (role: 'owner' | 'manager' | 'staff') => ({ id: 'me', role })
  it('owner 不能被移除或退出', () => {
    expect(canRemove(me('owner'), member({ user_id: 'me', role: 'owner' }))).toBe(false)
    expect(canRemove(me('manager'), member({ role: 'owner' }))).toBe(false)
  })
  it('owner 能移除 manager 與 staff', () => {
    expect(canRemove(me('owner'), member({ role: 'manager' }))).toBe(true)
    expect(canRemove(me('owner'), member({ role: 'staff' }))).toBe(true)
  })
  it('manager 只能移除 staff', () => {
    expect(canRemove(me('manager'), member({ role: 'staff' }))).toBe(true)
    expect(canRemove(me('manager'), member({ role: 'manager' }))).toBe(false)
  })
  it('staff 不能移除別人,但可以自己退出', () => {
    expect(canRemove(me('staff'), member({ role: 'staff' }))).toBe(false)
    expect(canRemove(me('staff'), member({ user_id: 'me', role: 'staff' }))).toBe(true)
  })
  it('canEditSchedule:管理者以上,或本人', () => {
    expect(canEditSchedule(me('owner'), 'x')).toBe(true)
    expect(canEditSchedule(me('manager'), 'x')).toBe(true)
    expect(canEditSchedule(me('staff'), 'x')).toBe(false)
    expect(canEditSchedule(me('staff'), 'me')).toBe(true)
  })
})

describe('各角色看到的選單與頁面', () => {
  beforeEach(() => {
    mockBookings([])
  })

  it('owner / manager 有稽核與方案', async () => {
    for (const role of ['owner', 'manager'] as const) {
      mockShopMe(role)
      const { unmount } = renderApp('/admin/demo-salon/bookings')
      const nav = await screen.findByRole('navigation', { name: '後台選單' })
      expect(within(nav).getByRole('link', { name: '稽核' })).toBeInTheDocument()
      expect(within(nav).getByRole('link', { name: '方案' })).toBeInTheDocument()
      unmount()
    }
  })

  it('staff 沒有稽核與方案的入口', async () => {
    mockShopMe('staff')
    renderApp('/admin/demo-salon/bookings')
    const nav = await screen.findByRole('navigation', { name: '後台選單' })
    expect(within(nav).getByRole('link', { name: '預約' })).toBeInTheDocument()
    expect(within(nav).queryByRole('link', { name: '稽核' })).not.toBeInTheDocument()
    expect(within(nav).queryByRole('link', { name: '方案' })).not.toBeInTheDocument()
  })

  it('staff 直接輸入稽核 / 方案的網址 → 說明沒有權限,而且完全不會打那兩支 API', async () => {
    mockShopMe('staff')
    let hits = 0
    server.use(
      http.get(`${API}/t/demo-salon/audit-logs`, () => ((hits += 1), error(403, 'x'))),
      http.get(`${API}/t/demo-salon/plan`, () => ((hits += 1), error(403, 'x'))),
    )
    for (const path of ['audit', 'plan']) {
      const { unmount } = renderApp(`/admin/demo-salon/${path}`)
      expect(await screen.findByRole('heading', { name: '沒有權限' })).toBeInTheDocument()
      unmount()
    }
    expect(hits).toBe(0)
  })

  it('不是這家店的成員(404)→ 說明並提供回到店家列表', async () => {
    server.use(http.get(`${API}/t/demo-salon/me`, () => error(404, '找不到資源')))
    renderApp('/admin/demo-salon')
    expect(await screen.findByText('找不到這家店')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: '回到我的店家' })).toBeInTheDocument()
  })
})

describe('預約列表', () => {
  beforeEach(() => mockShopMe('owner'))

  it('顯示店家時區的時間、顧客資料、狀態', async () => {
    mockBookings([booking()])
    renderApp('/admin/demo-salon/bookings?date=2026-10-05')
    const row = (await screen.findByText('王小明')).closest('li')!
    expect(within(row).getByText('14:00 – 15:00')).toBeInTheDocument()
    expect(within(row).getByText('已確認')).toBeInTheDocument()
    expect(within(row).getByRole('link', { name: 'ming@customer.example.com' })).toHaveAttribute(
      'href',
      'mailto:ming@customer.example.com',
    )
    expect(within(row).getByRole('link', { name: '0912-345-678' })).toHaveAttribute(
      'href',
      'tel:0912-345-678',
    )
  })

  it('依日期檢視 → 查詢範圍是店家當地那一天(台北 00:00 起算,不是 UTC)', async () => {
    const s = spy()
    server.use(
      http.get(`${API}/t/demo-salon/bookings`, async ({ request }) => {
        await s.record(request)
        return HttpResponse.json({ items: [], limit: 100, offset: 0 })
      }),
    )
    renderApp('/admin/demo-salon/bookings?date=2026-10-05')
    await screen.findByText('這一天沒有預約。')
    const q = s.calls[0].url.searchParams
    expect(q.get('from')).toBe('2026-10-04T16:00:00.000Z')
    expect(q.get('to')).toBe('2026-10-05T16:00:00.000Z')
  })

  it('空狀態依檢視不同', async () => {
    mockBookings([])
    renderApp('/admin/demo-salon/bookings?view=pending')
    expect(await screen.findByText('沒有待確認的預約。')).toBeInTheDocument()
  })

  it('網址裡亂填的日期 / 檢視不會壞掉,退回預設', async () => {
    mockBookings([])
    renderApp('/admin/demo-salon/bookings?date=not-a-date&view=hacked')
    expect(await screen.findByText('這一天沒有預約。')).toBeInTheDocument()
  })

  it('取消預約:要先確認,確認後才送出 PATCH', async () => {
    mockBookings([booking()])
    const s = spy<{ status?: string }>()
    server.use(
      http.patch(`${API}/t/demo-salon/bookings/b1`, async ({ request }) => {
        await s.record(request)
        return HttpResponse.json({ id: 'b1', status: 'cancelled', notes: null })
      }),
    )
    renderApp('/admin/demo-salon/bookings?date=2026-10-05')
    const user = userEvent.setup()
    const row = (await screen.findByText('王小明')).closest('li')!
    await user.click(within(row).getByRole('button', { name: '取消' }))
    expect(s.calls).toHaveLength(0)
    await user.click(await screen.findByRole('button', { name: '確定取消' }))
    await waitFor(() => expect(s.calls).toHaveLength(1))
    expect(s.calls[0].body).toEqual({ status: 'cancelled' })
  })

  it('已經結束的預約才有「完成 / 未到」;還沒開始的沒有', async () => {
    mockBookings([
      booking({
        id: 'past',
        customer_name: '過去的',
        starts_at: taipei('2026-10-05', '08:00'),
        ends_at: taipei('2026-10-05', '09:00'),
      }),
      booking({
        id: 'future',
        customer_name: '未來的',
        starts_at: taipei('2026-10-05', '16:00'),
        ends_at: taipei('2026-10-05', '17:00'),
      }),
    ])
    renderApp('/admin/demo-salon/bookings?date=2026-10-05')
    const past = (await screen.findByText('過去的')).closest('li')!
    const future = screen.getByText('未來的').closest('li')!
    expect(within(past).getByRole('button', { name: '完成' })).toBeInTheDocument()
    expect(within(past).getByRole('button', { name: '未到' })).toBeInTheDocument()
    expect(within(future).queryByRole('button', { name: '完成' })).not.toBeInTheDocument()
  })

  it('已取消的預約沒有取消鈕', async () => {
    mockBookings([booking({ status: 'cancelled' })])
    renderApp('/admin/demo-salon/bookings?date=2026-10-05')
    const row = (await screen.findByText('王小明')).closest('li')!
    expect(within(row).queryByRole('button', { name: '取消' })).not.toBeInTheDocument()
    expect(within(row).getByText('已取消')).toBeInTheDocument()
  })

  it('操作被後端拒絕(409)→ 在列上顯示原因', async () => {
    mockBookings([
      booking({ starts_at: taipei('2026-10-05', '08:00'), ends_at: taipei('2026-10-05', '09:00') }),
    ])
    server.use(
      http.patch(`${API}/t/demo-salon/bookings/b1`, () => error(409, '此預約已取消或已結束')),
    )
    renderApp('/admin/demo-salon/bookings?date=2026-10-05')
    const user = userEvent.setup()
    const row = (await screen.findByText('王小明')).closest('li')!
    await user.click(within(row).getByRole('button', { name: '完成' }))
    expect(await within(row).findByRole('alert')).toHaveTextContent('此預約已取消或已結束')
  })

  it('備註:送出的是輸入的內容', async () => {
    mockBookings([booking({ notes: '舊備註' })])
    const s = spy<{ notes?: string }>()
    server.use(
      http.patch(`${API}/t/demo-salon/bookings/b1`, async ({ request }) => {
        await s.record(request)
        return HttpResponse.json({ id: 'b1', status: 'confirmed', notes: '新備註' })
      }),
    )
    renderApp('/admin/demo-salon/bookings?date=2026-10-05')
    const user = userEvent.setup()
    const row = (await screen.findByText('王小明')).closest('li')!
    expect(within(row).getByText(/舊備註/)).toBeInTheDocument()
    await user.click(within(row).getByRole('button', { name: '備註' }))
    const box = await screen.findByRole('textbox', { name: /備註/ })
    expect(box).toHaveValue('舊備註') // 開啟時帶入目前的備註
    await user.clear(box)
    await user.type(box, '新備註')
    await user.click(screen.getByRole('button', { name: '儲存' }))
    await waitFor(() => expect(s.calls).toHaveLength(1))
    expect(s.calls[0].body).toEqual({ notes: '新備註' })
  })
})

describe('服務', () => {
  const listing = (items: AdminService[]) =>
    server.use(
      http.get(`${API}/t/demo-salon/services`, () =>
        HttpResponse.json({ items, total: items.length, limit: 100, offset: 0 }),
      ),
    )

  it('staff 只能看,沒有新增 / 編輯 / 開關 / 刪除', async () => {
    mockShopMe('staff')
    listing([service()])
    renderApp('/admin/demo-salon/services')
    expect(await screen.findByText('剪髮')).toBeInTheDocument()
    expect(screen.getByText(/只有擁有者與管理者/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /新增服務/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('switch')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '刪除' })).not.toBeInTheDocument()
  })

  it('開關:送出相反的 active,而且螢幕閱讀器看得到狀態', async () => {
    mockShopMe('owner')
    listing([service()])
    server.use(
      http.get(`${API}/t/demo-salon/plan`, () =>
        HttpResponse.json({
          plan: {
            id: 'free',
            name: '免費版',
            price_cents: 0,
            max_staff: 2,
            max_services: 5,
            max_bookings_per_month: 50,
          },
          usage: { staff: 1, pending_invitations: 0, services: 1, bookings_this_month: 0 },
        }),
      ),
    )
    const s = spy<{ active?: boolean }>()
    server.use(
      http.patch(`${API}/t/demo-salon/services/s1`, async ({ request }) => {
        await s.record(request)
        return HttpResponse.json(service({ active: false }))
      }),
    )
    renderApp('/admin/demo-salon/services')
    const user = userEvent.setup()
    const toggle = await screen.findByRole('switch', { name: /剪髮.*啟用中/ })
    expect(toggle).toHaveAttribute('aria-checked', 'true')
    await user.click(toggle)
    await waitFor(() => expect(s.calls).toHaveLength(1))
    expect(s.calls[0].body).toEqual({ active: false })
  })

  it('新增:價格從「元」換成「分」送出;驗證不過不打 API', async () => {
    mockShopMe('owner')
    listing([])
    server.use(
      http.get(`${API}/t/demo-salon/plan`, () =>
        HttpResponse.json({
          plan: {
            id: 'free',
            name: '免費版',
            price_cents: 0,
            max_staff: 2,
            max_services: 5,
            max_bookings_per_month: 50,
          },
          usage: { staff: 1, pending_invitations: 0, services: 0, bookings_this_month: 0 },
        }),
      ),
    )
    const s = spy<Record<string, unknown>>()
    server.use(
      http.post(`${API}/t/demo-salon/services`, async ({ request }) => {
        await s.record(request)
        return HttpResponse.json(service(), { status: 201 })
      }),
    )
    renderApp('/admin/demo-salon/services')
    const user = userEvent.setup()
    await user.click((await screen.findAllByRole('button', { name: /新增服務/ }))[0])
    const dialog = await screen.findByRole('dialog')
    await user.click(within(dialog).getByRole('button', { name: '儲存' }))
    expect(within(dialog).getByText('請輸入服務名稱')).toBeInTheDocument()
    expect(s.calls).toHaveLength(0)

    await user.type(within(dialog).getByLabelText('服務名稱'), '護髮')
    const minutes = within(dialog).getByLabelText('時長(分鐘)')
    await user.clear(minutes)
    await user.type(minutes, '3')
    await user.click(within(dialog).getByRole('button', { name: '儲存' }))
    expect(within(dialog).getByText(/5–1440/)).toBeInTheDocument()

    await user.clear(minutes)
    await user.type(minutes, '45')
    const price = within(dialog).getByLabelText('價格(元)')
    await user.clear(price)
    await user.type(price, '800')
    await user.click(within(dialog).getByRole('button', { name: '儲存' }))
    await waitFor(() => expect(s.calls).toHaveLength(1))
    expect(s.calls[0].body).toEqual({ name: '護髮', duration_minutes: 45, price_cents: 80000 })
  })

  it('超過方案上限(402)→ 在對話框顯示後端訊息', async () => {
    mockShopMe('owner')
    listing([])
    server.use(
      http.get(`${API}/t/demo-salon/plan`, () =>
        HttpResponse.json({
          plan: {
            id: 'free',
            name: '免費版',
            price_cents: 0,
            max_staff: 2,
            max_services: 5,
            max_bookings_per_month: 50,
          },
          usage: { staff: 1, pending_invitations: 0, services: 5, bookings_this_month: 0 },
        }),
      ),
      http.post(`${API}/t/demo-salon/services`, () =>
        error(402, '已達「免費版」方案的服務項目數上限(5),請升級方案'),
      ),
    )
    renderApp('/admin/demo-salon/services')
    const user = userEvent.setup()
    await user.click((await screen.findAllByRole('button', { name: /新增服務/ }))[0])
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('服務名稱'), '第六項')
    await user.click(within(dialog).getByRole('button', { name: '儲存' }))
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('已達「免費版」')
  })

  it('刪除有預約紀錄的服務(409)→ 提示改用停用', async () => {
    mockShopMe('owner')
    listing([service()])
    server.use(
      http.get(`${API}/t/demo-salon/plan`, () =>
        HttpResponse.json({
          plan: {
            id: 'free',
            name: '免費版',
            price_cents: 0,
            max_staff: 2,
            max_services: 5,
            max_bookings_per_month: 50,
          },
          usage: { staff: 1, pending_invitations: 0, services: 1, bookings_this_month: 0 },
        }),
      ),
      http.delete(`${API}/t/demo-salon/services/s1`, () => error(409, '此服務已有預約,請改為停用')),
    )
    renderApp('/admin/demo-salon/services')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '刪除' }))
    // 確認對話框裡還有一顆「刪除」,限定在 dialog 裡找
    const dialog = await screen.findByRole('dialog')
    await user.click(within(dialog).getByRole('button', { name: '刪除' }))
    expect(await within(dialog).findByText('此服務已有預約,請改為停用')).toBeInTheDocument()
  })
})

describe('團隊', () => {
  const members = [
    member({ user_id: 'u-owner', name: '林美玲', email: 'owner@demo.example.com', role: 'owner' }),
    member({ user_id: 'u-mgr', name: '經理', role: 'manager' }),
    member({ user_id: 'u-staff', name: '小安', role: 'staff' }),
  ]
  const listMembers = () =>
    server.use(http.get(`${API}/t/demo-salon/members`, () => HttpResponse.json(members)))

  it('owner 可以改角色與移除;owner 自己那列沒有移除、也不能改角色', async () => {
    mockShopMe('owner')
    listMembers()
    server.use(http.get(`${API}/t/demo-salon/invitations`, () => HttpResponse.json([])))
    renderApp('/admin/demo-salon/team')
    // 「林美玲」也出現在側欄,先等成員列表專屬的內容,避免抓到側欄就提早往下走
    const mgr = (await screen.findByText('經理')).closest('li')!
    const owner = screen.getByText('owner@demo.example.com').closest('li')!
    expect(within(owner).queryByRole('button', { name: /移除|退出/ })).not.toBeInTheDocument()
    expect(within(owner).queryByRole('combobox')).not.toBeInTheDocument()
    expect(within(mgr).getByRole('combobox', { name: '經理 的角色' })).toBeInTheDocument()
    expect(within(mgr).getByRole('button', { name: '移除' })).toBeInTheDocument()
  })

  it('manager 看不到角色下拉選單,只能移除 staff', async () => {
    mockShopMe('manager')
    listMembers()
    server.use(http.get(`${API}/t/demo-salon/invitations`, () => HttpResponse.json([])))
    renderApp('/admin/demo-salon/team')
    await screen.findByText('小安')
    expect(screen.queryByRole('combobox')).not.toBeInTheDocument()
    expect(
      within(screen.getByText('小安').closest('li')!).getByRole('button', { name: '移除' }),
    ).toBeInTheDocument()
    expect(
      within(screen.getByText('經理').closest('li')!).queryByRole('button', { name: '移除' }),
    ).not.toBeInTheDocument()
  })

  it('改角色:送出選的角色', async () => {
    mockShopMe('owner')
    listMembers()
    server.use(http.get(`${API}/t/demo-salon/invitations`, () => HttpResponse.json([])))
    const s = spy<{ role?: string }>()
    server.use(
      http.patch(`${API}/t/demo-salon/members/u-staff`, async ({ request }) => {
        await s.record(request)
        return HttpResponse.json({ user_id: 'u-staff', role: 'manager' })
      }),
    )
    renderApp('/admin/demo-salon/team')
    const user = userEvent.setup()
    await user.selectOptions(
      await screen.findByRole('combobox', { name: '小安 的角色' }),
      'manager',
    )
    await waitFor(() => expect(s.calls).toHaveLength(1))
    expect(s.calls[0].body).toEqual({ role: 'manager' })
  })

  it('staff 看不到邀請區塊,而且不會打邀請清單 API', async () => {
    mockShopMe('staff')
    listMembers()
    let hits = 0
    server.use(http.get(`${API}/t/demo-salon/invitations`, () => ((hits += 1), error(403, 'x'))))
    renderApp('/admin/demo-salon/team')
    await screen.findByText('小安')
    expect(screen.queryByText('待接受的邀請')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '邀請成員' })).not.toBeInTheDocument()
    expect(hits).toBe(0)
  })

  it('邀請:Email 格式錯誤不送出;成功後顯示一次性連結(token 在 # 之後)', async () => {
    mockShopMe('owner')
    listMembers()
    server.use(http.get(`${API}/t/demo-salon/invitations`, () => HttpResponse.json([])))
    const s = spy<{ email?: string; role?: string }>()
    const token = 'a'.repeat(64)
    server.use(
      http.post(`${API}/t/demo-salon/invitations`, async ({ request }) => {
        await s.record(request)
        return HttpResponse.json(
          {
            id: 'i1',
            email: 'new@example.com',
            role: 'staff',
            expires_at: '2026-10-12T00:00:00Z',
            created_at: '2026-10-05T00:00:00Z',
            token,
          },
          { status: 201 },
        )
      }),
    )
    renderApp('/admin/demo-salon/team')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '邀請成員' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('對方的 Email'), 'not-an-email')
    await user.click(within(dialog).getByRole('button', { name: '寄出邀請' }))
    expect(within(dialog).getByText('Email 格式不正確')).toBeInTheDocument()
    expect(s.calls).toHaveLength(0)

    await user.clear(within(dialog).getByLabelText('對方的 Email'))
    await user.type(within(dialog).getByLabelText('對方的 Email'), ' new@example.com ')
    await user.click(within(dialog).getByRole('button', { name: '寄出邀請' }))
    const link = await within(dialog).findByLabelText('邀請連結')
    expect(s.calls[0].body).toEqual({ email: 'new@example.com', role: 'staff' })
    expect((link as HTMLInputElement).value).toMatch(
      new RegExp(`/invitations/accept#token=${token}$`),
    )
  })
})

describe('接受邀請', () => {
  const TOKEN = 'b'.repeat(64)
  it('沒有 token / 太短 → 說明連結不完整,不會打 API', async () => {
    let hits = 0
    server.use(http.post(`${API}/invitations/accept`, () => ((hits += 1), error(404, 'x'))))
    renderApp('/invitations/accept#token=short')
    expect(await screen.findByText('邀請連結不完整')).toBeInTheDocument()
    expect(hits).toBe(0)
  })

  it('未登入 → 要求先登入 / 註冊,而且登入後會回到這個邀請頁', async () => {
    localStorage.clear()
    const { clearSession } = await import('./session')
    clearSession()
    renderApp(`/invitations/accept#token=${TOKEN}`)
    const login = await screen.findByRole('link', { name: '登入' })
    expect(login).toHaveAttribute('href', '/admin/login')
  })

  it('已登入 → 顯示目前的 Email;成功後進入該店後台', async () => {
    const s = spy<{ token?: string }>()
    mockShopMe('staff')
    server.use(
      http.post(`${API}/invitations/accept`, async ({ request }) => {
        await s.record(request)
        return HttpResponse.json({ tenant_slug: 'demo-salon' })
      }),
      http.get(`${API}/t/demo-salon/bookings`, () =>
        HttpResponse.json({ items: [], limit: 100, offset: 0 }),
      ),
    )
    renderApp(`/invitations/accept#token=${TOKEN}`)
    const user = userEvent.setup()
    expect(await screen.findByText(USER.email)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '接受邀請' }))
    await screen.findByRole('navigation', { name: '後台選單' })
    expect(s.calls[0].body).toEqual({ token: TOKEN })
  })

  it('無效 / 過期 / Email 不符(後端一律 404)→ 白話說明可能的原因', async () => {
    server.use(http.post(`${API}/invitations/accept`, () => error(404, '找不到資源')))
    renderApp(`/invitations/accept#token=${TOKEN}`)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '接受邀請' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('已過期')
  })

  it('成員人數已達方案上限(402)→ 顯示後端訊息', async () => {
    server.use(
      http.post(`${API}/invitations/accept`, () =>
        error(402, '此店家的成員人數已達方案上限,請聯絡店家'),
      ),
    )
    renderApp(`/invitations/accept#token=${TOKEN}`)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '接受邀請' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('方案上限')
  })
})

describe('方案與稽核頁', () => {
  it('用量條:滿了顯示無法再新增,不限額不顯示條', async () => {
    mockShopMe('owner')
    server.use(
      http.get(`${API}/t/demo-salon/plan`, () =>
        HttpResponse.json({
          plan: {
            id: 'free',
            name: '免費版',
            price_cents: 0,
            max_staff: 2,
            max_services: null,
            max_bookings_per_month: 50,
          },
          usage: { staff: 1, pending_invitations: 1, services: 9, bookings_this_month: 10 },
        }),
      ),
      http.get(`${API}/plans`, () => HttpResponse.json([])),
    )
    renderApp('/admin/demo-salon/plan')
    expect(await screen.findByText('已達上限,無法再新增。')).toBeInTheDocument()
    // 成員 = 1 位成員 + 1 個待接受的邀請(邀請會預留名額)
    expect(screen.getByText('2 / 2')).toBeInTheDocument()
    expect(screen.getByText('9 / 不限')).toBeInTheDocument()
    expect(screen.getAllByRole('meter')).toHaveLength(2) // 不限額的那一項沒有條
  })

  it('稽核:翻成白話、系統取消顯示原因、可載入更多', async () => {
    mockShopMe('owner')
    const entry = (id: number, action: string, detail = {}) => ({
      id,
      created_at: '2026-10-05T02:00:00Z',
      actor_type: 'user',
      actor_user_id: 'u',
      actor_name: '林美玲',
      action,
      entity_type: 'x',
      entity_id: null,
      detail,
    })
    const calls: (string | null)[] = []
    server.use(
      http.get(`${API}/t/demo-salon/audit-logs`, ({ request }) => {
        const before = new URL(request.url).searchParams.get('before')
        calls.push(before)
        return HttpResponse.json(
          before
            ? { items: [entry(1, 'service.created')], next_before: null }
            : {
                items: [
                  entry(3, 'member.role_changed', { from: 'staff', to: 'manager' }),
                  {
                    ...entry(2, 'booking.cancelled', { reason: 'confirmation_expired' }),
                    actor_type: 'system',
                    actor_user_id: null,
                    actor_name: null,
                  },
                ],
                next_before: 2,
              },
        )
      }),
    )
    renderApp('/admin/demo-salon/audit')
    const user = userEvent.setup()
    expect(await screen.findByText('變更角色')).toBeInTheDocument()
    expect(screen.getByText('員工 → 管理者')).toBeInTheDocument()
    expect(screen.getByText('超過期限未確認')).toBeInTheDocument()
    expect(screen.getByText('系統')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '載入更多' }))
    expect(await screen.findByText('新增服務')).toBeInTheDocument()
    expect(calls).toEqual([null, '2'])
    expect(screen.queryByRole('button', { name: '載入更多' })).not.toBeInTheDocument()
  })
})

describe('其他', () => {
  it('SHOP fixture 的時區就是預約頁的時區', () => {
    expect(SHOP().timezone).toBe('Asia/Taipei')
  })
})
