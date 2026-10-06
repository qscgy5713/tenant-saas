import { act, createEvent, fireEvent, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { HttpResponse, http } from 'msw'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { freezeNow } from '../test/freeze'
import { renderApp } from '../test/render'
import { API, error, server, standardSlots, taipei } from '../test/server'
import { SHOP, USER, loginAs, mockShopMe, preloadAdmin, spy } from '../test/admin'
import * as redirect from '../lib/redirect'
import { saveFile } from '../lib/download'
import { canDeactivate, canEditSchedule, canReactivate, canRemove } from './permissions'
import type { AdminBooking, AdminService, Member } from './types'

// 下載檔案:jsdom 沒有 URL.createObjectURL,而且測試只需要知道「存了什麼檔」
vi.mock('../lib/download', () => ({ saveFile: vi.fn() }))

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
  active: true,
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
  it('canDeactivate:規則同 canRemove,而且對象要是啟用中;可以停用自己(退出)', () => {
    expect(canDeactivate(me('owner'), member({ role: 'staff' }))).toBe(true)
    expect(canDeactivate(me('owner'), member({ role: 'staff', active: false }))).toBe(false)
    expect(canDeactivate(me('owner'), member({ user_id: 'me', role: 'owner' }))).toBe(false)
    expect(canDeactivate(me('manager'), member({ role: 'manager' }))).toBe(false)
    expect(canDeactivate(me('staff'), member({ role: 'staff' }))).toBe(false)
    expect(canDeactivate(me('staff'), member({ user_id: 'me', role: 'staff' }))).toBe(true)
  })
  it('canReactivate:只有管理者以上;對象要是停用中;不是自己', () => {
    const gone = (over: Partial<Member> = {}) => member({ role: 'staff', active: false, ...over })
    expect(canReactivate(me('owner'), gone())).toBe(true)
    expect(canReactivate(me('manager'), gone())).toBe(true)
    expect(canReactivate(me('manager'), gone({ role: 'manager' }))).toBe(false)
    expect(canReactivate(me('staff'), gone())).toBe(false)
    expect(canReactivate(me('owner'), member({ role: 'staff' }))).toBe(false) // 本來就啟用
    expect(canReactivate(me('owner'), gone({ user_id: 'me' }))).toBe(false)
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

describe('頁面開著跨過午夜(櫃檯的平板整夜不關)', () => {
  beforeEach(() => mockShopMe('owner'))
  // 凍結在台北 10/5 10:00;「過了午夜」= 時間來到台北 10/6 00:30,再讓視窗取得焦點(休眠醒來 / 切回分頁)
  const pastMidnight = () =>
    act(() => {
      vi.setSystemTime(new Date('2026-10-05T16:30:00Z'))
      window.dispatchEvent(new Event('focus'))
    })

  it('總覽:「今天的行程」換成新的一天,而且用新的一天重新查詢', async () => {
    const froms: (string | null)[] = []
    server.use(
      http.get(`${API}/t/demo-salon/bookings`, ({ request }) => {
        froms.push(new URL(request.url).searchParams.get('from'))
        return HttpResponse.json({ items: [], limit: 100, offset: 0 })
      }),
    )
    renderApp('/admin/demo-salon')
    expect(await screen.findByText(/10月5日/)).toBeInTheDocument()
    expect(froms).toContain('2026-10-04T16:00:00.000Z') // 10/5 的 00:00(台北)

    pastMidnight()
    expect(await screen.findByText(/10月6日/)).toBeInTheDocument()
    expect(screen.queryByText(/10月5日/)).not.toBeInTheDocument()
    await waitFor(() => expect(froms).toContain('2026-10-05T16:00:00.000Z')) // 10/6 的 00:00
  })

  it('預約頁:沒指定日期時跟著「今天」走;自己選過日期就不會被拉走', async () => {
    mockBookings([])
    const { unmount } = renderApp('/admin/demo-salon/bookings')
    expect(await screen.findByLabelText('日期')).toHaveValue('2026-10-05')
    pastMidnight()
    await waitFor(() => expect(screen.getByLabelText('日期')).toHaveValue('2026-10-06'))
    unmount()

    vi.setSystemTime(new Date('2026-10-05T02:00:00Z'))
    renderApp('/admin/demo-salon/bookings?date=2026-10-05')
    expect(await screen.findByLabelText('日期')).toHaveValue('2026-10-05')
    pastMidnight()
    await screen.findByText('這一天沒有預約。')
    expect(screen.getByLabelText('日期')).toHaveValue('2026-10-05')
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

describe('員工替顧客改期', () => {
  beforeEach(() => mockShopMe('owner'))

  const slotsFor = (from: string, to: string) => ({
    timezone: 'Asia/Taipei',
    slots: standardSlots(from, to, ['u-owner']),
  })

  it('只有已確認、還沒開始的預約有「改期」;待確認 / 已取消 / 已過去的沒有', async () => {
    mockBookings([
      booking({
        id: 'ok',
        customer_name: '顧客乙',
        starts_at: taipei('2026-10-05', '16:00'),
        ends_at: taipei('2026-10-05', '17:00'),
      }),
      booking({
        id: 'p',
        customer_name: '顧客丙',
        status: 'pending',
        starts_at: taipei('2026-10-05', '16:00'),
        ends_at: taipei('2026-10-05', '17:00'),
      }),
      booking({
        id: 'c',
        customer_name: '顧客丁',
        status: 'cancelled',
        starts_at: taipei('2026-10-05', '16:00'),
        ends_at: taipei('2026-10-05', '17:00'),
      }),
      booking({
        id: 'past',
        customer_name: '顧客戊',
        starts_at: taipei('2026-10-05', '08:00'),
        ends_at: taipei('2026-10-05', '09:00'),
      }),
    ])
    renderApp('/admin/demo-salon/bookings?date=2026-10-05')
    const has = async (name: string) =>
      within((await screen.findByText(name)).closest('li')!).queryByRole('button', { name: '改期' })
    expect(await has('顧客乙')).toBeInTheDocument()
    expect(await has('顧客丙')).not.toBeInTheDocument()
    expect(await has('顧客丁')).not.toBeInTheDocument()
    expect(await has('顧客戊')).not.toBeInTheDocument()
  })

  it('時段來自「依預約」的端點(排除自己),選好後送出新時間並重新整理列表', async () => {
    let list = [
      booking({ starts_at: taipei('2026-10-05', '16:00'), ends_at: taipei('2026-10-05', '17:00') }),
    ]
    server.use(
      http.get(`${API}/t/demo-salon/bookings`, () =>
        HttpResponse.json({ items: list, limit: 100, offset: 0 }),
      ),
    )
    let shopWide = 0
    const s = spy<{ start?: string }>()
    server.use(
      http.get(`${API}/t/demo-salon/bookings/b1/availability`, ({ request }) => {
        const q = new URL(request.url).searchParams
        return HttpResponse.json(slotsFor(q.get('from')!, q.get('to')!))
      }),
      http.get(
        `${API}/public/shops/demo-salon/availability`,
        () => ((shopWide += 1), HttpResponse.json({ timezone: 'Asia/Taipei', slots: [] })),
      ),
      http.post(`${API}/t/demo-salon/bookings/b1/reschedule`, async ({ request }) => {
        await s.record(request)
        list = [
          booking({
            starts_at: taipei('2026-10-06', '14:00'),
            ends_at: taipei('2026-10-06', '15:00'),
          }),
        ]
        return HttpResponse.json({
          id: 'b1',
          starts_at: taipei('2026-10-06', '14:00'),
          ends_at: taipei('2026-10-06', '15:00'),
        })
      }),
    )
    renderApp('/admin/demo-salon/bookings?date=2026-10-05')
    const user = userEvent.setup()
    const row = (await screen.findByText('王小明')).closest('li')!
    await user.click(within(row).getByRole('button', { name: '改期' }))
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText(/通知信給顧客/)).toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: '請先選擇新時間' })).toBeDisabled()

    await user.click(await within(dialog).findByRole('button', { name: '14:00' }))
    await user.click(within(dialog).getByRole('button', { name: /改到.*14:00/ }))
    await waitFor(() => expect(s.calls).toHaveLength(1))
    expect(s.calls[0].body).toEqual({ start: taipei('2026-10-05', '14:00') })
    expect(shopWide).toBe(0)
  })

  it('額度不足(402)或時段被占走(409)→ 在對話框顯示後端的原因,不關閉', async () => {
    mockBookings([
      booking({ starts_at: taipei('2026-10-05', '16:00'), ends_at: taipei('2026-10-05', '17:00') }),
    ])
    server.use(
      http.get(`${API}/t/demo-salon/bookings/b1/availability`, ({ request }) => {
        const q = new URL(request.url).searchParams
        return HttpResponse.json(slotsFor(q.get('from')!, q.get('to')!))
      }),
      http.post(`${API}/t/demo-salon/bookings/b1/reschedule`, () =>
        error(402, '已達「免費版」方案的每月預約數上限(50),請升級方案'),
      ),
    )
    renderApp('/admin/demo-salon/bookings?date=2026-10-05')
    const user = userEvent.setup()
    await user.click(
      within((await screen.findByText('王小明')).closest('li')!).getByRole('button', {
        name: '改期',
      }),
    )
    const dialog = await screen.findByRole('dialog')
    await user.click(await within(dialog).findByRole('button', { name: '14:00' }))
    await user.click(within(dialog).getByRole('button', { name: /改到/ }))
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('每月預約數上限')
    expect(dialog).toHaveAttribute('open')
  })
})

describe('週日曆', () => {
  beforeEach(() => mockShopMe('owner'))

  it('畫出整週的預約(不含已取消);點區塊開啟詳情;位置用 class 而不是行內 style', async () => {
    mockBookings([
      booking({
        id: 'mon',
        customer_name: '週一客',
        starts_at: taipei('2026-10-05', '16:00'),
        ends_at: taipei('2026-10-05', '17:00'),
      }),
      booking({
        id: 'sun',
        customer_name: '週日客',
        starts_at: taipei('2026-10-11', '15:00'),
        ends_at: taipei('2026-10-11', '16:00'),
      }),
      booking({ id: 'gone', customer_name: '取消客', status: 'cancelled' }),
    ])
    renderApp('/admin/demo-salon/bookings?view=week&date=2026-10-07')
    const user = userEvent.setup()
    const cal = await screen.findByRole('region', { name: '週日曆' })
    const mon = await within(cal).findByRole('button', { name: /週一客/ })
    expect(within(cal).getByRole('button', { name: /週日客/ })).toBeInTheDocument()
    expect(within(cal).queryByText(/取消客/)).not.toBeInTheDocument()

    // 16:00 相對於 9 點起點 = 28 格;1 小時 = 4 格。不能有行內 style(正式環境 CSP 會擋)
    expect(mon.parentElement).toHaveClass('cal-top-28', 'cal-len-4', 'cal-lane-0-of-1')
    expect(cal.querySelectorAll('[style]')).toHaveLength(0)

    await user.click(mon)
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText('週一客')).toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: '改期' })).toBeInTheDocument()
  })

  describe('拖曳改期', () => {
    // jsdom 沒有排版:欄位 10 小時 × 每小時 48px = 480px(預設顯示 9–19 點),區塊的上緣依 class 的格數(每格 12px)
    beforeEach(() => {
      vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (
        this: Element,
      ) {
        const top = this.classList.contains('cal-col')
          ? 0
          : Number(/cal-top-(\d+)/.exec(this.className)?.[1] ?? 0) * 12
        const height = this.classList.contains('cal-col') ? 480 : 48
        return {
          top,
          height,
          bottom: top + height,
          left: 0,
          right: 0,
          width: 0,
          x: 0,
          y: top,
        } as DOMRect
      })
    })
    afterEach(() => vi.restoreAllMocks())

    // jsdom 沒有 DragEvent,事件會退化成一般 Event、沒有 clientY → 自己補上
    function drag(el: Element, type: 'dragStart' | 'dragOver' | 'drop', clientY: number) {
      const data = { setData: vi.fn(), effectAllowed: '', dropEffect: '' }
      const event = createEvent[type](el, { dataTransfer: data })
      Object.defineProperty(event, 'clientY', { value: clientY })
      fireEvent(el, event)
      return { event, data }
    }

    const columns = (cal: HTMLElement) => cal.querySelectorAll<HTMLElement>('.cal-col')

    async function setup(over: Partial<AdminBooking> = {}) {
      mockBookings([
        booking({
          starts_at: taipei('2026-10-05', '16:00'),
          ends_at: taipei('2026-10-05', '17:00'),
          ...over,
        }),
      ])
      renderApp('/admin/demo-salon/bookings?view=week&date=2026-10-07')
      const cal = await screen.findByRole('region', { name: '週日曆' })
      const item = (await within(cal).findByRole('button', { name: /王小明/ })).parentElement!
      return { cal, item }
    }

    it('拖到另一天的別個時間 → 先確認 → 以放開位置(吸附 15 分鐘、扣掉抓取點)送出改期', async () => {
      const { cal, item } = await setup()
      const s = spy<{ start?: string }>()
      server.use(
        http.post(`${API}/t/demo-salon/bookings/b1/reschedule`, async ({ request }) => {
          await s.record(request)
          return HttpResponse.json({ id: 'b1', starts_at: '', ends_at: '' })
        }),
      )
      const user = userEvent.setup()
      expect(item).toHaveAttribute('draggable', 'true')

      // 區塊上緣在 336px(16:00),抓在往下 24px 處;放到週二欄的 14:00(240px)+ 抓取點 + 5px 誤差
      const start = drag(item, 'dragStart', 336 + 24)
      // Firefox 沒有 setData 就不會開始拖曳;沒有 preventDefault 的 dragover 則不允許放開
      expect(start.data.setData).toHaveBeenCalled()
      const tuesday = columns(cal)[1]
      const over = drag(tuesday, 'dragOver', 240 + 24 + 5)
      expect(over.event.defaultPrevented).toBe(true)
      expect(tuesday).toHaveClass('cal-col-over')
      drag(tuesday, 'drop', 240 + 24 + 5)

      const dialog = await screen.findByRole('dialog')
      expect(s.calls).toHaveLength(0) // 還沒確認,不能送出
      expect(within(dialog).getByText(/通知信給顧客/)).toBeInTheDocument()
      expect(within(dialog).getByText(/14:00/)).toBeInTheDocument()
      await user.click(within(dialog).getByRole('button', { name: '確定改期' }))

      await waitFor(() => expect(s.calls).toHaveLength(1))
      expect(s.calls[0].body).toEqual({ start: taipei('2026-10-06', '14:00') })
      expect(await screen.findByText('已改期,並寄出通知信給顧客。')).toBeInTheDocument()
      expect(tuesday).not.toHaveClass('cal-col-over')
    })

    it('不是我們發起的拖曳(例如從桌面拖檔案進來)→ 不接受放開', async () => {
      const { cal } = await setup()
      const over = drag(columns(cal)[1], 'dragOver', 100)
      expect(over.event.defaultPrevented).toBe(false)
      drag(columns(cal)[1], 'drop', 100)
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    })

    it('放回原位不會跳出確認;確認視窗按取消不送出', async () => {
      const { cal, item } = await setup()
      let posted = 0
      server.use(
        http.post(
          `${API}/t/demo-salon/bookings/b1/reschedule`,
          () => ((posted += 1), HttpResponse.json({})),
        ),
      )
      const user = userEvent.setup()

      drag(item, 'dragStart', 336 + 10)
      drag(columns(cal)[0], 'drop', 336 + 10) // 週一原本的位置
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()

      drag(item, 'dragStart', 336)
      drag(columns(cal)[2], 'drop', 336)
      const dialog = await screen.findByRole('dialog')
      await user.click(within(dialog).getByRole('button', { name: '取消' }))
      await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
      expect(posted).toBe(0)
    })

    it('拖出視窗會夾在第一格 / 最後一格(不會變成負的或隔天)', async () => {
      const { cal, item } = await setup()
      const user = userEvent.setup()
      const starts: (string | undefined)[] = []
      server.use(
        http.post(`${API}/t/demo-salon/bookings/b1/reschedule`, async ({ request }) => {
          starts.push(((await request.json()) as { start?: string }).start)
          return HttpResponse.json({})
        }),
      )
      for (const [y, expected] of [
        [-500, taipei('2026-10-06', '09:00')],
        [9999, taipei('2026-10-06', '18:45')],
      ] as const) {
        drag(item, 'dragStart', 336)
        drag(columns(cal)[1], 'drop', y + 336)
        const dialog = await screen.findByRole('dialog')
        await user.click(within(dialog).getByRole('button', { name: '確定改期' }))
        await waitFor(() => expect(starts.at(-1)).toBe(expected))
        await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
      }
    })

    it('後端拒絕(例如員工沒空)→ 原因顯示在確認視窗裡,不關閉', async () => {
      const { cal, item } = await setup()
      server.use(
        http.post(`${API}/t/demo-salon/bookings/b1/reschedule`, () =>
          error(409, '這個時段已被預約'),
        ),
      )
      const user = userEvent.setup()
      drag(item, 'dragStart', 336)
      drag(columns(cal)[1], 'drop', 240)
      const dialog = await screen.findByRole('dialog')
      await user.click(within(dialog).getByRole('button', { name: '確定改期' }))
      expect(await within(dialog).findByRole('alert')).toHaveTextContent('這個時段已被預約')
      expect(dialog).toHaveAttribute('open')
    })

    it('只有「已確認且還沒開始」的預約能拖;待確認 / 已過去的不行', async () => {
      mockBookings([
        booking({
          id: 'ok',
          customer_name: '可拖客',
          starts_at: taipei('2026-10-05', '16:00'),
          ends_at: taipei('2026-10-05', '17:00'),
        }),
        booking({
          id: 'pend',
          customer_name: '待確認客',
          status: 'pending',
          starts_at: taipei('2026-10-06', '16:00'),
          ends_at: taipei('2026-10-06', '17:00'),
        }),
        booking({
          id: 'past',
          customer_name: '過去客',
          starts_at: taipei('2026-10-05', '09:00'),
          ends_at: taipei('2026-10-05', '10:00'),
        }),
      ])
      renderApp('/admin/demo-salon/bookings?view=week&date=2026-10-07')
      const cal = await screen.findByRole('region', { name: '週日曆' })
      const draggable = async (name: RegExp) =>
        (await within(cal).findByRole('button', { name })).parentElement!.getAttribute('draggable')
      expect(await draggable(/可拖客/)).toBe('true')
      expect(await draggable(/待確認客/)).toBe('false')
      expect(await draggable(/過去客/)).toBe('false')
    })
  })

  it('一週超過一頁(100 筆)也會抓完,並用上一週 / 下一週切換', async () => {
    const offsets: string[] = []
    const many = Array.from({ length: 130 }, (_, i) =>
      booking({
        id: `m${i}`,
        customer_name: `客${i}`,
        starts_at: taipei('2026-10-06', '10:00'),
        ends_at: taipei('2026-10-06', '11:00'),
      }),
    )
    server.use(
      http.get(`${API}/t/demo-salon/bookings`, ({ request }) => {
        const q = new URL(request.url).searchParams
        offsets.push(`${q.get('offset')}`)
        const offset = Number(q.get('offset') ?? 0)
        return HttpResponse.json({ items: many.slice(offset, offset + 100), limit: 100, offset })
      }),
    )
    renderApp('/admin/demo-salon/bookings?view=week&date=2026-10-07')
    const user = userEvent.setup()
    const cal = await screen.findByRole('region', { name: '週日曆' })
    await waitFor(() => expect(within(cal).getAllByRole('button')).toHaveLength(130))
    expect(offsets).toEqual(['0', '100'])

    await user.click(screen.getByRole('button', { name: '下一週' }))
    await waitFor(() => expect(screen.getByLabelText('日期')).toHaveValue('2026-10-14'))
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

  describe('停用 / 重新啟用', () => {
    const roster = (leftActive = true) => [
      member({
        user_id: 'u-owner',
        name: '林美玲',
        email: 'owner@demo.example.com',
        role: 'owner',
      }),
      member({ user_id: 'u-staff', name: '小安', role: 'staff', active: leftActive }),
    ]
    const serve = (list: () => Member[]) =>
      server.use(
        http.get(`${API}/t/demo-salon/members`, () => HttpResponse.json(list())),
        http.get(`${API}/t/demo-salon/invitations`, () => HttpResponse.json([])),
      )

    it('owner 停用員工:先確認 → 送出 → 列表標示「已停用」,角色下拉消失,出現「重新啟用」', async () => {
      mockShopMe('owner')
      let active = true
      serve(() => roster(active))
      const s = spy<unknown>()
      server.use(
        http.post(`${API}/t/demo-salon/members/u-staff/deactivate`, async ({ request }) => {
          await s.record(request)
          active = false
          return new HttpResponse(null, { status: 204 })
        }),
      )
      renderApp('/admin/demo-salon/team')
      const user = userEvent.setup()
      const row = () => screen.getByText('小安').closest('li')!
      await screen.findByText('小安')
      expect(within(row()).getByRole('combobox', { name: '小安 的角色' })).toBeInTheDocument()

      await user.click(within(row()).getByRole('button', { name: '停用' }))
      const dialog = await screen.findByRole('dialog')
      expect(s.calls).toHaveLength(0) // 還沒確認
      expect(within(dialog).getByText(/歷史預約與紀錄會保留/)).toBeInTheDocument()
      await user.click(within(dialog).getByRole('button', { name: '停用' }))

      await waitFor(() => expect(within(row()).getByText('已停用')).toBeInTheDocument())
      expect(s.calls).toHaveLength(1)
      expect(within(row()).queryByRole('combobox')).not.toBeInTheDocument()
      expect(within(row()).queryByRole('button', { name: '停用' })).not.toBeInTheDocument()
      expect(within(row()).getByRole('button', { name: '重新啟用' })).toBeInTheDocument()
      expect(within(row()).getByRole('button', { name: '移除' })).toBeInTheDocument()
    })

    it('還有未來已確認的預約 → 後端拒絕,原因顯示在確認視窗裡,視窗不關', async () => {
      mockShopMe('owner')
      serve(() => roster())
      server.use(
        http.post(`${API}/t/demo-salon/members/u-staff/deactivate`, () =>
          error(409, '此成員還有 2 筆未來已確認的預約,請先取消(顧客會收到通知)或請顧客改期'),
        ),
      )
      renderApp('/admin/demo-salon/team')
      const user = userEvent.setup()
      await user.click(
        within((await screen.findByText('小安')).closest('li')!).getByRole('button', {
          name: '停用',
        }),
      )
      const dialog = await screen.findByRole('dialog')
      await user.click(within(dialog).getByRole('button', { name: '停用' }))
      expect(await within(dialog).findByRole('alert')).toHaveTextContent(
        '還有 2 筆未來已確認的預約',
      )
      expect(dialog).toHaveAttribute('open')
      expect(screen.queryByText('已停用')).not.toBeInTheDocument()
    })

    it('重新啟用:送出後恢復;名額不足(402)時顯示原因', async () => {
      mockShopMe('owner')
      let active = false
      serve(() => roster(active))
      let mode: 'full' | 'ok' = 'full'
      const s = spy<unknown>()
      server.use(
        http.post(`${API}/t/demo-salon/members/u-staff/reactivate`, async ({ request }) => {
          await s.record(request)
          if (mode === 'full') return error(402, '已達「免費版」方案的成員人數上限(2),請升級方案')
          active = true
          return new HttpResponse(null, { status: 204 })
        }),
      )
      renderApp('/admin/demo-salon/team')
      const user = userEvent.setup()
      const row = () => screen.getByText('小安').closest('li')!
      await user.click(
        await within((await screen.findByText('小安')).closest('li')!).findByRole('button', {
          name: '重新啟用',
        }),
      )
      expect(await screen.findByText(/成員人數上限/)).toBeInTheDocument()
      expect(within(row()).getByText('已停用')).toBeInTheDocument()

      mode = 'ok'
      await user.click(within(row()).getByRole('button', { name: '重新啟用' }))
      await waitFor(() => expect(within(row()).queryByText('已停用')).not.toBeInTheDocument())
      expect(s.calls).toHaveLength(2)
    })

    it('manager:員工有「停用」與「移除」,同階的管理者兩個都沒有;staff 只能「退出團隊」自己', async () => {
      mockShopMe('manager')
      serve(() => [...roster(), member({ user_id: 'u-mgr2', name: '另一位經理', role: 'manager' })])
      renderApp('/admin/demo-salon/team')
      await screen.findByText('小安')
      const staffRow = screen.getByText('小安').closest('li')!
      expect(within(staffRow).getByRole('button', { name: '停用' })).toBeInTheDocument()
      expect(within(staffRow).getByRole('button', { name: '移除' })).toBeInTheDocument()
      const peer = screen.getByText('另一位經理').closest('li')!
      expect(
        within(peer).queryByRole('button', { name: /停用|移除|重新啟用/ }),
      ).not.toBeInTheDocument()
    })

    it('自己「退出團隊」= 停用自己(保留歷史),成功後回到店家列表', async () => {
      const me = { id: 'u-me', email: 'me@demo.example.com', name: '小我' }
      loginAs(me)
      mockShopMe('staff')
      serve(() => [
        member({
          user_id: 'u-owner',
          name: '林美玲',
          email: 'owner@demo.example.com',
          role: 'owner',
        }),
        member({ user_id: me.id, name: me.name, email: me.email, role: 'staff' }),
      ])
      const s = spy<unknown>()
      let removed = 0
      server.use(
        http.post(`${API}/t/demo-salon/members/${me.id}/deactivate`, async ({ request }) => {
          await s.record(request)
          return new HttpResponse(null, { status: 204 })
        }),
        http.delete(
          `${API}/t/demo-salon/members/${me.id}`,
          () => ((removed += 1), error(409, 'x')),
        ),
        http.get(`${API}/tenants`, () => HttpResponse.json([])),
      )
      renderApp('/admin/demo-salon/team')
      const user = userEvent.setup()
      const mine = (await screen.findByText(me.email)).closest('li')!
      expect(within(mine).queryByRole('button', { name: '移除' })).not.toBeInTheDocument()
      await user.click(within(mine).getByRole('button', { name: '退出團隊' }))
      const dialog = await screen.findByRole('dialog')
      expect(within(dialog).getByText(/管理者可以讓你重新加入/)).toBeInTheDocument()
      await user.click(within(dialog).getByRole('button', { name: '退出' }))
      expect(await screen.findByRole('heading', { name: '我的店家' })).toBeInTheDocument()
      expect(s.calls).toHaveLength(1)
      expect(removed).toBe(0)
    })
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

describe('月曆', () => {
  beforeEach(() => mockShopMe('owner'))

  const at = (
    day: string,
    hhmm: string,
    id: string,
    name: string,
    over: Partial<AdminBooking> = {},
  ) =>
    booking({
      id,
      customer_name: name,
      starts_at: taipei(day, hhmm),
      ends_at: taipei(day, `${String(Number(hhmm.slice(0, 2)) + 1).padStart(2, '0')}:00`),
      ...over,
    })

  it('整個月(含前後補滿的日子)的預約依日期放進格子;已取消不畫;一格最多 3 筆其餘「還有 N 筆」', async () => {
    mockBookings([
      at('2026-10-02', '10:00', 'a', '甲'),
      at('2026-10-02', '11:00', 'b', '乙'),
      at('2026-10-02', '13:00', 'c', '丙'),
      at('2026-10-02', '15:00', 'd', '丁'),
      at('2026-10-02', '16:00', 'x', '取消客', { status: 'cancelled' }),
      at('2026-09-30', '09:00', 'e', '戊'), // 補在月初前面的日子
    ])
    renderApp('/admin/demo-salon/bookings?view=month&date=2026-10-15')
    const cal = await screen.findByRole('region', { name: '月曆' })
    expect(await within(cal).findByRole('button', { name: /甲/ })).toBeInTheDocument()
    expect(within(cal).getByRole('button', { name: /丙/ })).toBeInTheDocument()
    expect(within(cal).queryByRole('button', { name: /丁/ })).not.toBeInTheDocument() // 第 4 筆被收起來
    expect(within(cal).getByRole('button', { name: '還有 1 筆' })).toBeInTheDocument()
    expect(within(cal).getByRole('button', { name: /戊/ })).toBeInTheDocument()
    expect(within(cal).queryByText(/取消客/)).not.toBeInTheDocument()
    expect(cal.querySelectorAll('[style]')).toHaveLength(0)
    expect(screen.getByRole('heading', { name: '2026年10月' })).toBeInTheDocument()
  })

  it('點預約開詳情;點日期或「還有 N 筆」回到當天的列表', async () => {
    mockBookings([at('2026-10-06', '16:00', 'a', '甲客')])
    renderApp('/admin/demo-salon/bookings?view=month&date=2026-10-15')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: /甲客/ }))
    expect(within(await screen.findByRole('dialog')).getByText('甲客')).toBeInTheDocument()
    await user.keyboard('{Escape}')

    await user.click(screen.getByRole('button', { name: /^2026-10-06/ }))
    expect(await screen.findByRole('heading', { name: /10月6日/ })).toBeInTheDocument()
    expect(screen.getByLabelText('日期')).toHaveValue('2026-10-06')
    expect(screen.queryByRole('region', { name: '月曆' })).not.toBeInTheDocument()
  })

  it('上個月 / 下個月:跨年也正確,並以該月第一天查詢範圍', async () => {
    const ranges: { from: string | null; to: string | null }[] = []
    server.use(
      http.get(`${API}/t/demo-salon/bookings`, ({ request }) => {
        const p = new URL(request.url).searchParams
        ranges.push({ from: p.get('from'), to: p.get('to') })
        return HttpResponse.json({ items: [], limit: 100, offset: 0 })
      }),
    )
    renderApp('/admin/demo-salon/bookings?view=month&date=2026-12-20')
    const user = userEvent.setup()
    expect(await screen.findByRole('heading', { name: '2026年12月' })).toBeInTheDocument()
    // 2026/12 的月曆從 11/30(週一)到 2027/1/3(週日)
    expect(ranges[0]).toEqual({
      from: taipei('2026-11-30', '00:00'),
      to: taipei('2027-01-04', '00:00'),
    })

    await user.click(screen.getByRole('button', { name: '下個月' }))
    expect(await screen.findByRole('heading', { name: '2027年1月' })).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '上個月' }))
    await user.click(screen.getByRole('button', { name: '上個月' }))
    expect(await screen.findByRole('heading', { name: '2026年11月' })).toBeInTheDocument()
  })
})

describe('顧客頁', () => {
  const customer = (over: object = {}) => ({
    id: 'c1',
    name: '王小明',
    email: 'ming@customer.example.com',
    phone: '0912-345-678',
    bookings: 3,
    completed: 2,
    no_shows: 1,
    last_at: taipei('2026-09-20', '10:00'),
    next_at: taipei('2026-10-12', '14:00'),
    ...over,
  })

  describe('刪除顧客資料', () => {
    const history = [
      {
        id: 'h1',
        status: 'completed',
        starts_at: taipei('2026-09-20', '10:00'),
        ends_at: taipei('2026-09-20', '11:00'),
        notes: null,
        service_name: '剪髮',
        staff_name: '林美玲',
      },
    ]
    const serve = (name = '王小明') => {
      let erased = false
      server.use(
        http.get(`${API}/t/demo-salon/customers`, () =>
          HttpResponse.json({
            items: [customer({ name: erased ? '已刪除的顧客' : name })],
            limit: 30,
            offset: 0,
          }),
        ),
        http.get(`${API}/t/demo-salon/customers/c1`, () =>
          HttpResponse.json({ ...customer({ name: erased ? '已刪除的顧客' : name }), history }),
        ),
      )
      return () => (erased = true)
    }
    const open = async (role: 'owner' | 'manager') => {
      mockShopMe(role)
      renderApp('/admin/demo-salon/customers')
      const user = userEvent.setup()
      await user.click(await screen.findByRole('button', { name: /王小明|已刪除的顧客/ }))
      return { user, dialog: await screen.findByRole('dialog') }
    }

    it('只有擁有者看得到「刪除顧客資料」;管理者沒有', async () => {
      serve()
      const { dialog } = await open('manager')
      await within(dialog).findByText('預約紀錄')
      expect(within(dialog).queryByRole('button', { name: '刪除顧客資料' })).not.toBeInTheDocument()
    })

    it('先說明後果(不可復原),確認後送出,清單重新載入、視窗關閉', async () => {
      const markErased = serve()
      const s = spy()
      server.use(
        http.post(`${API}/t/demo-salon/customers/c1/anonymize`, async ({ request }) => {
          await s.record(request)
          markErased()
          return new HttpResponse(null, { status: 204 })
        }),
      )
      const { user, dialog } = await open('owner')
      await user.click(await within(dialog).findByRole('button', { name: '刪除顧客資料' }))
      const confirm = (await screen.findAllByRole('dialog')).at(-1)!
      expect(within(confirm).getByText(/無法復原/)).toBeInTheDocument()
      expect(within(confirm).getByText(/預約紀錄本身會保留/)).toBeInTheDocument()
      expect(s.calls).toHaveLength(0) // 還沒確認不能送出
      await user.click(within(confirm).getByRole('button', { name: '永久刪除' }))

      await waitFor(() => expect(s.calls).toHaveLength(1))
      expect(await screen.findByRole('button', { name: '已刪除的顧客' })).toBeInTheDocument()
      expect(screen.queryByRole('button', { name: '王小明' })).not.toBeInTheDocument()
    })

    it('還有未來的預約 → 後端拒絕,原因顯示在確認視窗,資料沒動', async () => {
      serve()
      server.use(
        http.post(`${API}/t/demo-salon/customers/c1/anonymize`, () =>
          error(409, '這位顧客還有 1 筆未來或待確認的預約,請先取消再刪除資料'),
        ),
      )
      const { user, dialog } = await open('owner')
      await user.click(await within(dialog).findByRole('button', { name: '刪除顧客資料' }))
      const confirm = (await screen.findAllByRole('dialog')).at(-1)!
      await user.click(within(confirm).getByRole('button', { name: '永久刪除' }))
      expect(await within(confirm).findByRole('alert')).toHaveTextContent('還有 1 筆未來')
      expect(confirm).toHaveAttribute('open')
    })

    it('已經刪除過的顧客不再顯示刪除按鈕', async () => {
      serve('已刪除的顧客')
      const { dialog } = await open('owner')
      await within(dialog).findByText('預約紀錄')
      expect(within(dialog).queryByRole('button', { name: '刪除顧客資料' })).not.toBeInTheDocument()
    })
  })

  it('員工看不到;管理者看到清單、未到標記與下次預約', async () => {
    mockShopMe('staff')
    renderApp('/admin/demo-salon/customers')
    expect(await screen.findByRole('heading', { name: '沒有權限' })).toBeInTheDocument()
  })

  it('搜尋(停止輸入後才查)、載入更多、點名字看預約紀錄', async () => {
    mockShopMe('manager')
    const queries: { q: string | null; offset: string | null }[] = []
    const many = Array.from({ length: 30 }, (_, i) => customer({ id: `c${i}`, name: `顧客${i}` }))
    server.use(
      http.get(`${API}/t/demo-salon/customers`, ({ request }) => {
        const p = new URL(request.url).searchParams
        queries.push({ q: p.get('q'), offset: p.get('offset') })
        const offset = Number(p.get('offset') ?? 0)
        if (p.get('q') === '小明')
          return HttpResponse.json({ items: [customer()], limit: 30, offset })
        return HttpResponse.json({
          items: offset === 0 ? many : [customer({ id: 'last', name: '最後一位' })],
          limit: 30,
          offset,
        })
      }),
      http.get(`${API}/t/demo-salon/customers/c1`, () =>
        HttpResponse.json({
          ...customer(),
          history: [
            {
              id: 'h1',
              status: 'completed',
              starts_at: taipei('2026-09-20', '10:00'),
              ends_at: taipei('2026-09-20', '11:00'),
              notes: '怕癢',
              service_name: '剪髮',
              staff_name: '林美玲',
            },
            {
              id: 'h2',
              status: 'no_show',
              starts_at: taipei('2026-08-01', '10:00'),
              ends_at: taipei('2026-08-01', '11:00'),
              notes: null,
              service_name: '染髮',
              staff_name: '林美玲',
            },
          ],
        }),
      ),
    )
    renderApp('/admin/demo-salon/customers')
    const user = userEvent.setup()
    expect(await screen.findByText('顧客0')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: '載入更多' }))
    expect(await screen.findByText('最後一位')).toBeInTheDocument()
    expect(queries.map((x) => x.offset)).toEqual(['0', '30'])

    // 一個字一個字輸入只會查最後的結果(防抖)
    await user.type(screen.getByLabelText('搜尋顧客'), '小明')
    await waitFor(() => expect(queries.at(-1)?.q).toBe('小明'))
    expect(queries.filter((x) => x.q).map((x) => x.q)).toEqual(['小明'])
    await waitFor(() => expect(screen.queryByText('顧客0')).not.toBeInTheDocument())

    expect(screen.getByText('未到 1')).toBeInTheDocument()
    expect(screen.getByText('共 3 次預約')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '王小明' }))
    const dialog = await screen.findByRole('dialog')
    expect(await within(dialog).findByText('染髮')).toBeInTheDocument()
    expect(within(dialog).getByText('備註:怕癢')).toBeInTheDocument()
    expect(within(dialog).getByText('未到')).toBeInTheDocument()
  })

  it('沒有顧客 / 搜尋沒結果 → 不同的說明', async () => {
    mockShopMe('owner')
    server.use(
      http.get(`${API}/t/demo-salon/customers`, () =>
        HttpResponse.json({ items: [], limit: 30, offset: 0 }),
      ),
    )
    renderApp('/admin/demo-salon/customers')
    const user = userEvent.setup()
    expect(await screen.findByText(/還沒有顧客/)).toBeInTheDocument()
    await user.type(screen.getByLabelText('搜尋顧客'), 'zzz')
    expect(await screen.findByText('沒有符合的顧客。')).toBeInTheDocument()
  })
})

describe('建立店家', () => {
  const open = async () => {
    server.use(http.get(`${API}/tenants`, () => HttpResponse.json([])))
    renderApp('/admin')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '建立店家' }))
    return { user, dialog: await screen.findByRole('dialog') }
  }

  it('名稱或代稱沒填 → 前端先擋,不打 API', async () => {
    let posted = 0
    server.use(http.post(`${API}/tenants`, () => ((posted += 1), HttpResponse.json({}))))
    const { user, dialog } = await open()
    await user.click(within(dialog).getByRole('button', { name: '建立店家' }))
    expect(await within(dialog).findAllByText(/請/)).not.toHaveLength(0)
    expect(posted).toBe(0)
  })

  it('已達店家數量上限(402)→ 顯示後端的原因,視窗不關,不會導頁', async () => {
    server.use(
      http.post(`${API}/tenants`, () => error(402, '每位使用者最多可以建立 5 家店,已達上限')),
    )
    const { user, dialog } = await open()
    await user.type(within(dialog).getByLabelText('店家名稱'), '第六家店')
    await user.type(within(dialog).getByLabelText(/預約頁網址代稱/), 'sixth-shop')
    await user.click(within(dialog).getByRole('button', { name: '建立店家' }))
    expect(await within(dialog).findByText(/最多可以建立 5 家店/)).toBeInTheDocument()
    expect(dialog).toHaveAttribute('open')
    expect(screen.getByRole('heading', { name: '我的店家' })).toBeInTheDocument()
  })

  it('送出名稱、代稱(自動轉小寫)與時區', async () => {
    const s = spy<{ name?: string; slug?: string; timezone?: string }>()
    server.use(
      http.post(`${API}/tenants`, async ({ request }) => {
        await s.record(request)
        return HttpResponse.json({ ...SHOP(), slug: 'new-salon', name: '新店' }, { status: 201 })
      }),
      http.get(`${API}/t/new-salon/me`, () => HttpResponse.json({ ...SHOP(), slug: 'new-salon' })),
    )
    const { user, dialog } = await open()
    await user.type(within(dialog).getByLabelText('店家名稱'), '  新店  ')
    await user.type(within(dialog).getByLabelText(/預約頁網址代稱/), 'New-Salon')
    await user.click(within(dialog).getByRole('button', { name: '建立店家' }))
    await waitFor(() => expect(s.calls).toHaveLength(1))
    expect(s.calls[0].body).toEqual({ name: '新店', slug: 'new-salon', timezone: 'Asia/Taipei' })
  })
})

describe('店家設定', () => {
  it('只有店主看得到「設定」;管理者直接開網址會看到沒有權限', async () => {
    mockShopMe('manager')
    renderApp('/admin/demo-salon/settings')
    expect(await screen.findByRole('heading', { name: '沒有權限' })).toBeInTheDocument()
    expect(screen.queryByRole('link', { name: '設定' })).not.toBeInTheDocument()
  })

  it('只送出有改的欄位;改時區時警告營業時間會改用新時區解讀;儲存後提示', async () => {
    mockShopMe('owner')
    const s = spy<{ name?: string; timezone?: string }>()
    let current = SHOP('owner')
    server.use(
      http.get(`${API}/t/demo-salon/me`, () => HttpResponse.json(current)),
      http.patch(`${API}/t/demo-salon`, async ({ request }) => {
        const body = (await request.clone().json()) as { name?: string; timezone?: string }
        await s.record(request)
        current = { ...current, ...body }
        return HttpResponse.json(current)
      }),
    )
    renderApp('/admin/demo-salon/settings')
    const user = userEvent.setup()
    const name = await screen.findByLabelText('店家名稱')
    const save = screen.getByRole('button', { name: '儲存' })
    expect(save).toBeDisabled() // 沒改任何東西
    expect(screen.queryByText(/不會移動/)).not.toBeInTheDocument()

    await user.clear(name)
    await user.type(name, '  新名字 ')
    await user.click(save)
    await waitFor(() => expect(s.calls).toHaveLength(1))
    expect(s.calls[0].body).toEqual({ name: '新名字' }) // 時區沒改就不送
    expect(await screen.findByText('已儲存。')).toBeInTheDocument()

    await user.selectOptions(screen.getByLabelText('店家所在時區'), 'Asia/Tokyo')
    expect(screen.getByText(/不會移動/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '儲存' }))
    await waitFor(() => expect(s.calls).toHaveLength(2))
    expect(s.calls[1].body).toEqual({ timezone: 'Asia/Tokyo' })
  })

  it('名稱空白 → 前端擋下;後端錯誤顯示原因', async () => {
    mockShopMe('owner')
    const s = spy()
    server.use(
      http.patch(`${API}/t/demo-salon`, async ({ request }) => {
        await s.record(request)
        return error(400, '不支援的時區')
      }),
    )
    renderApp('/admin/demo-salon/settings')
    const user = userEvent.setup()
    const name = await screen.findByLabelText('店家名稱')
    await user.clear(name)
    await user.click(screen.getByRole('button', { name: '儲存' }))
    expect(await screen.findByText(/店家名稱/, { selector: '.field-msg' })).toBeInTheDocument()
    expect(s.calls).toHaveLength(0)

    await user.type(name, '森林系髮廊2')
    await user.click(screen.getByRole('button', { name: '儲存' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('不支援的時區')
  })
})

describe('線上付款(Stripe)', () => {
  const FREE = {
    id: 'free',
    name: '免費版',
    price_cents: 0,
    max_staff: 2,
    max_services: 5,
    max_bookings_per_month: 50,
  }
  const PRO = {
    id: 'pro',
    name: '專業版',
    price_cents: 49900,
    max_staff: 10,
    max_services: 50,
    max_bookings_per_month: 1000,
  }
  const usage = { staff: 1, pending_invitations: 0, services: 1, bookings_this_month: 0 }
  const billing = (over = {}) => ({
    enabled: true,
    purchasable: ['pro'],
    subscription: null,
    can_checkout: true,
    can_manage: false,
    ...over,
  })
  const mockPlan = (current = FREE) =>
    server.use(
      http.get(`${API}/t/demo-salon/plan`, () => HttpResponse.json({ plan: current, usage })),
      http.get(`${API}/plans`, () => HttpResponse.json([FREE, PRO])),
    )

  it('可訂閱的方案顯示「升級」,按下後導向 Stripe 結帳頁;目前方案與不可買的沒有按鈕', async () => {
    mockShopMe('owner')
    mockPlan()
    const go = vi.spyOn(redirect, 'redirectTo').mockImplementation(() => {})
    const s = spy<{ plan?: string }>()
    server.use(
      http.get(`${API}/t/demo-salon/billing`, () => HttpResponse.json(billing())),
      http.post(`${API}/t/demo-salon/billing/checkout`, async ({ request }) => {
        await s.record(request)
        return HttpResponse.json({ url: 'https://checkout.stripe.com/pay/abc' })
      }),
    )
    renderApp('/admin/demo-salon/plan')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '升級到專業版' }))
    await waitFor(() => expect(go).toHaveBeenCalledWith('https://checkout.stripe.com/pay/abc'))
    expect(s.calls[0].body).toEqual({ plan: 'pro' })
    expect(screen.queryByRole('button', { name: '升級到免費版' })).not.toBeInTheDocument()
    expect(screen.queryByText(/尚未開放/)).not.toBeInTheDocument()
    go.mockRestore()
  })

  it('未啟用線上付款 → 沒有升級按鈕,顯示聯絡管理員的說明', async () => {
    mockShopMe('owner')
    mockPlan() // 預設的 billing 回應是 enabled: false
    renderApp('/admin/demo-salon/plan')
    expect(await screen.findByText(/線上升級與付款尚未開放/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /升級到/ })).not.toBeInTheDocument()
  })

  it('非店主:不查付款資料、沒有任何付款按鈕', async () => {
    mockShopMe('manager')
    mockPlan()
    let hits = 0
    server.use(http.get(`${API}/t/demo-salon/billing`, () => ((hits += 1), error(403, '沒有權限'))))
    renderApp('/admin/demo-salon/plan')
    expect(await screen.findByText(/升級或變更方案請聯絡店家擁有者/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /升級到|管理訂閱/ })).not.toBeInTheDocument()
    expect(hits).toBe(0)
  })

  it('訂閱中:顯示續訂日與「管理訂閱」,不能再升級(避免重複訂閱);按下導向客戶入口', async () => {
    mockShopMe('owner')
    mockPlan(PRO)
    const go = vi.spyOn(redirect, 'redirectTo').mockImplementation(() => {})
    server.use(
      http.get(`${API}/t/demo-salon/billing`, () =>
        HttpResponse.json(
          billing({
            can_checkout: false,
            can_manage: true,
            subscription: {
              status: 'active',
              current_period_end: '2030-03-05T00:00:00Z',
              cancel_at_period_end: false,
            },
          }),
        ),
      ),
      http.post(`${API}/t/demo-salon/billing/portal`, () =>
        HttpResponse.json({ url: 'https://billing.stripe.com/p/xyz' }),
      ),
    )
    renderApp('/admin/demo-salon/plan')
    const user = userEvent.setup()
    expect(await screen.findByText(/訂閱中,下次續訂日/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /升級到/ })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '管理訂閱與付款方式' }))
    await waitFor(() => expect(go).toHaveBeenCalledWith('https://billing.stripe.com/p/xyz'))
    go.mockRestore()
  })

  it('付款失敗(past_due)與期末取消都有明確提示', async () => {
    mockShopMe('owner')
    mockPlan(PRO)
    const sub = (over: object) =>
      billing({
        can_checkout: false,
        can_manage: true,
        subscription: {
          status: 'active',
          current_period_end: '2030-03-05T00:00:00Z',
          cancel_at_period_end: false,
          ...over,
        },
      })
    server.use(
      http.get(`${API}/t/demo-salon/billing`, () => HttpResponse.json(sub({ status: 'past_due' }))),
    )
    const first = renderApp('/admin/demo-salon/plan')
    expect(await screen.findByText(/付款失敗,請更新付款方式/)).toBeInTheDocument()
    first.unmount()

    server.use(
      http.get(`${API}/t/demo-salon/billing`, () =>
        HttpResponse.json(sub({ cancel_at_period_end: true })),
      ),
    )
    renderApp('/admin/demo-salon/plan')
    expect(await screen.findByText(/到期後取消/)).toBeInTheDocument()
  })

  it('結帳後導回(?checkout=success):方案還沒生效時顯示處理中,並輪詢到 webhook 生效', async () => {
    mockShopMe('owner')
    let polls = 0
    server.use(
      http.get(`${API}/t/demo-salon/plan`, () => {
        polls += 1
        return HttpResponse.json({ plan: polls >= 3 ? PRO : FREE, usage })
      }),
      http.get(`${API}/plans`, () => HttpResponse.json([FREE, PRO])),
    )
    renderApp('/admin/demo-salon/plan?checkout=success')
    expect(await screen.findByText(/方案正在更新/)).toBeInTheDocument()
    // 方案是等 webhook 才生效的,導回的網址參數本身不能當作已付款
    expect(
      await screen.findByText('已升級為「專業版」。', undefined, { timeout: 8000 }),
    ).toBeInTheDocument()
    expect(polls).toBeGreaterThanOrEqual(3)
  }, 15000)

  it('取消結帳(?checkout=cancel)→ 說明方案沒變;付款服務失敗(502)顯示錯誤', async () => {
    mockShopMe('owner')
    mockPlan()
    server.use(
      http.get(`${API}/t/demo-salon/billing`, () => HttpResponse.json(billing())),
      http.post(`${API}/t/demo-salon/billing/checkout`, () =>
        error(502, '付款服務暫時無法使用,請稍後再試'),
      ),
    )
    renderApp('/admin/demo-salon/plan?checkout=cancel')
    const user = userEvent.setup()
    expect(await screen.findByText('已取消付款,方案沒有變更。')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '升級到專業版' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('伺服器發生問題') // 共用 API 層把 5xx 統一成這句
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

describe('匯出預約', () => {
  beforeEach(() => vi.mocked(saveFile).mockClear())
  const URL_ = `${API}/t/demo-salon/bookings/export.csv`
  const csv = () =>
    new HttpResponse('\ufeff預約編號\r\n', {
      headers: {
        'content-type': 'text/csv; charset=utf-8',
        'content-disposition': 'attachment; filename="bookings-demo-salon-20261005.csv"',
      },
    })

  it('員工沒有「匯出 CSV」(不能一次帶走全店的顧客資料);管理者有', async () => {
    mockShopMe('staff')
    mockBookings([])
    const { unmount } = renderApp('/admin/demo-salon/bookings')
    await screen.findByRole('button', { name: /新增預約/ })
    expect(screen.queryByRole('button', { name: '匯出 CSV' })).not.toBeInTheDocument()
    unmount()

    mockShopMe('manager')
    renderApp('/admin/demo-salon/bookings')
    expect(await screen.findByRole('button', { name: '匯出 CSV' })).toBeInTheDocument()
  })

  it('送出日期範圍(店家時區的整天)→ 存檔;視窗提醒內含個資', async () => {
    mockShopMe('owner')
    mockBookings([])
    const s = spy()
    server.use(
      http.get(URL_, async ({ request }) => {
        await s.record(request)
        return csv()
      }),
    )
    renderApp('/admin/demo-salon/bookings')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '匯出 CSV' }))
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText(/姓名、Email 與電話/)).toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: '匯出' }))
    expect(await within(dialog).findByText(/已匯出/)).toBeInTheDocument()

    const q = s.calls[0].url.searchParams
    expect(q.get('from')).toBe('2026-09-05T16:00:00.000Z')
    expect(q.get('to')).toBe('2026-10-05T16:00:00.000Z')
    expect(vi.mocked(saveFile).mock.calls[0][1]).toBe('bookings-demo-salon-20261005.csv')
  })

  it('筆數超過上限 → 顯示後端的說明,不存檔', async () => {
    mockShopMe('owner')
    mockBookings([])
    server.use(
      http.get(URL_, () => error(400, '符合條件的預約超過 50000 筆,請縮小日期範圍後再匯出')),
    )
    renderApp('/admin/demo-salon/bookings')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '匯出 CSV' }))
    const dialog = await screen.findByRole('dialog')
    await user.click(within(dialog).getByRole('button', { name: '匯出' }))
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('超過 50000 筆')
    expect(saveFile).not.toHaveBeenCalled()
  })
})

describe('稽核日誌匯出', () => {
  let listCalls = 0
  beforeEach(() => {
    mockShopMe('owner')
    vi.mocked(saveFile).mockClear()
    listCalls = 0
    server.use(
      http.get(`${API}/t/demo-salon/audit-logs`, () => {
        listCalls += 1
        return HttpResponse.json({ items: [], next_before: null })
      }),
    )
  })
  const EXPORT_URL = `${API}/t/demo-salon/audit-logs/export.csv`
  const csv = (name = 'audit-demo-salon-20261005.csv') =>
    new HttpResponse('\ufeffid\r\n1\r\n', {
      headers: {
        'content-type': 'text/csv; charset=utf-8',
        'content-disposition': `attachment; filename="${name}"`,
      },
    })

  async function openDialog(path = '/admin/demo-salon/audit') {
    renderApp(path)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '匯出 CSV' }))
    return { user, dialog: await screen.findByRole('dialog') }
  }

  it('預設匯出最近 30 天(店家時區的整天,含起訖兩天);下載的檔名取自後端', async () => {
    const s = spy()
    server.use(
      http.get(EXPORT_URL, async ({ request }) => {
        await s.record(request)
        return csv('audit-demo-salon-20261005.csv')
      }),
    )
    const { user, dialog } = await openDialog()
    expect(within(dialog).getByLabelText('開始日期')).toHaveValue('2026-09-06')
    expect(within(dialog).getByLabelText('結束日期')).toHaveValue('2026-10-05')
    await user.click(within(dialog).getByRole('button', { name: '匯出' }))

    expect(await within(dialog).findByText(/已匯出/)).toBeInTheDocument()
    // 匯出本身會留一筆稽核紀錄:列表要重新載入,使用者才看得到剛剛那筆
    expect(listCalls).toBeGreaterThan(1)
    const q = s.calls[0].url.searchParams
    expect(q.get('from')).toBe('2026-09-05T16:00:00.000Z') // 9/6 00:00(台北)
    expect(q.get('to')).toBe('2026-10-05T16:00:00.000Z') // 10/6 00:00 → 10/5 整天都算
    expect(q.get('action')).toBeNull()
    expect(saveFile).toHaveBeenCalledTimes(1)
    const [blob, filename] = vi.mocked(saveFile).mock.calls[0]
    expect(filename).toBe('audit-demo-salon-20261005.csv')
    expect(await (blob as Blob).text()).toContain('id')
  })

  it('沿用目前的類別篩選,而且在視窗裡告訴使用者', async () => {
    const s = spy()
    server.use(
      http.get(EXPORT_URL, async ({ request }) => {
        await s.record(request)
        return csv()
      }),
    )
    renderApp('/admin/demo-salon/audit')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '預約' }))
    await user.click(screen.getByRole('button', { name: '匯出 CSV' }))
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText('預約')).toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: '匯出' }))
    await within(dialog).findByText(/已匯出/)
    expect(s.calls[0].url.searchParams.get('action')).toBe('booking.')
  })

  it('筆數超過上限 → 顯示後端的說明,不下載任何東西,改完日期可以再試', async () => {
    let attempts = 0
    server.use(
      http.get(EXPORT_URL, () =>
        ++attempts === 1 ? error(400, '符合條件的紀錄超過 50000 筆,請縮小日期範圍後再匯出') : csv(),
      ),
    )
    const { user, dialog } = await openDialog()
    await user.click(within(dialog).getByRole('button', { name: '匯出' }))
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('超過 50000 筆')
    expect(saveFile).not.toHaveBeenCalled()
    expect(dialog).toHaveAttribute('open')

    await user.clear(within(dialog).getByLabelText('開始日期'))
    await user.type(within(dialog).getByLabelText('開始日期'), '2026-10-01')
    await user.click(within(dialog).getByRole('button', { name: '匯出' }))
    expect(await within(dialog).findByText(/已匯出/)).toBeInTheDocument()
    expect(saveFile).toHaveBeenCalledTimes(1)
  })

  it('日期沒填 / 結束早於開始 → 前端先擋,不打 API', async () => {
    let hits = 0
    server.use(http.get(EXPORT_URL, () => ((hits += 1), csv())))
    const { user, dialog } = await openDialog()
    await user.clear(within(dialog).getByLabelText('結束日期'))
    await user.click(within(dialog).getByRole('button', { name: '匯出' }))
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('請選擇日期範圍')

    await user.type(within(dialog).getByLabelText('結束日期'), '2026-08-01')
    await user.click(within(dialog).getByRole('button', { name: '匯出' }))
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('結束日期不能早於開始日期')
    expect(hits).toBe(0)
  })

  it('連不上伺服器 → 顯示錯誤,不會存出一個壞檔', async () => {
    server.use(http.get(EXPORT_URL, () => HttpResponse.error()))
    const { user, dialog } = await openDialog()
    await user.click(within(dialog).getByRole('button', { name: '匯出' }))
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('無法連線')
    expect(saveFile).not.toHaveBeenCalled()
  })
})

describe('其他', () => {
  it('SHOP fixture 的時區就是預約頁的時區', () => {
    expect(SHOP().timezone).toBe('Asia/Taipei')
  })
})
