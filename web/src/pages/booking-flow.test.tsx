import { act, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { HttpResponse, http } from 'msw'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { freezeNow } from '../test/freeze'
import { renderApp } from '../test/render'
import {
  API,
  SERVICE,
  STAFF_A,
  STAFF_B,
  error,
  mockShop,
  server,
  standardSlots,
  taipei,
  today,
} from '../test/server'
import { addDays } from '../lib/time'
import { telHref } from '../lib/phone'

beforeEach(() => freezeNow())

const BOOK = '/s/demo-salon/book/svc-cut'

describe('服務列表', () => {
  it('顯示服務、時長與價格,連到預約頁', async () => {
    mockShop({
      services: [SERVICE, { id: 's2', name: '洗髮護理', duration_minutes: 30, price_cents: 0 }],
    })
    renderApp('/s/demo-salon')
    const cut = await screen.findByRole('link', { name: /剪髮/ })
    expect(cut).toHaveTextContent('1 小時')
    expect(cut).toHaveTextContent('500')
    expect(cut).toHaveAttribute('href', '/s/demo-salon/book/svc-cut')
    expect(screen.getByRole('link', { name: /洗髮護理/ })).toHaveTextContent('免費')
  })

  it('找不到店家 → 明確的訊息,而且不提供沒意義的「重試」', async () => {
    server.use(http.get(`${API}/public/shops/nope`, () => error(404, '找不到資源')))
    renderApp('/s/nope')
    expect(await screen.findByText('找不到這家店')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '重試' })).not.toBeInTheDocument()
  })

  it('伺服器錯誤 → 顯示錯誤代碼並提供重試', async () => {
    server.use(http.get(`${API}/public/shops/demo-salon`, () => error(500, 'boom', 'req-77')))
    renderApp('/s/demo-salon')
    expect(await screen.findByText(/req-77/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '重試' })).toBeInTheDocument()
  })

  it('沒有任何服務 → 空狀態', async () => {
    mockShop({ services: [] })
    renderApp('/s/demo-salon')
    expect(await screen.findByText('目前沒有開放預約的服務。')).toBeInTheDocument()
  })
})

describe('選日期與時間', () => {
  it('自動選到最早有空的一天;沒有空檔的日期停用並標示', async () => {
    mockShop()
    renderApp(BOOK)
    await screen.findByRole('button', { name: '10:00' })
    // 今天是週一 10/5:週日 10/11 不營業
    const sunday = await screen.findByRole('button', { name: /10月11日.*沒有空檔/ })
    expect(sunday).toBeDisabled()
    const monday = screen.getByRole('button', { name: /10月5日\s?週一/ })
    expect(monday).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: '10:00' })).toBeInTheDocument()
    expect(screen.getByText(/GMT\+8/)).toBeInTheDocument()
  })

  it('頁面開著跨過午夜 → 日期列改從新的「今天」開始,舊的一天消失(不會讓人選到已經過去的日子)', async () => {
    mockShop()
    renderApp(BOOK)
    const strip = () => screen.getByRole('group', { name: '日期' })
    await screen.findByRole('button', { name: '10:00' })
    expect(within(strip()).getByRole('button', { name: /10月5日\s?週一/ })).toBeInTheDocument()

    // 台北 10/6 00:30:休眠醒來 / 切回分頁
    act(() => {
      vi.setSystemTime(new Date('2026-10-05T16:30:00Z'))
      window.dispatchEvent(new Event('focus'))
    })
    await waitFor(() =>
      expect(within(strip()).queryByRole('button', { name: /10月5日/ })).not.toBeInTheDocument(),
    )
    expect(await within(strip()).findByRole('button', { name: /10月6日\s?週二/ })).toHaveAttribute(
      'aria-pressed',
      'true',
    )
    expect(await screen.findByRole('button', { name: '10:00' })).toBeInTheDocument()
  })

  it('使用者已經翻到後面幾週 / 選了某一天,這時跨過午夜 → 回到從新的「今天」開始(翻頁數是相對於今天的)', async () => {
    mockShop()
    renderApp(BOOK)
    const strip = () => screen.getByRole('group', { name: '日期' })
    const user = userEvent.setup()
    await screen.findByRole('button', { name: '10:00' })

    // 翻到下一週:日期列從 10/12 開始
    await user.click(screen.getByRole('button', { name: '下一週' }))
    expect(await within(strip()).findByRole('button', { name: /10月12日/ })).toBeInTheDocument()
    expect(within(strip()).queryByRole('button', { name: /10月5日/ })).not.toBeInTheDocument()

    // 跨過午夜:如果頁數(1)原封不動,新的「今天」是 10/6,下一週會變成 10/13。
    // 正確的是整個重來:回到第一頁,從 10/6 開始
    act(() => {
      vi.setSystemTime(new Date('2026-10-05T16:30:00Z'))
      window.dispatchEvent(new Event('focus'))
    })
    expect(
      await within(strip()).findByRole('button', { name: /10月6日\s?週二/ }),
    ).toBeInTheDocument()
    expect(within(strip()).queryByRole('button', { name: /10月13日/ })).not.toBeInTheDocument()
  })

  it('時段載入期間,日期是中性狀態,不能先被當成「沒有空檔」', async () => {
    mockShop()
    // 手動控制回應時機,不靠碰運氣
    let release!: () => void
    const gate = new Promise<void>((resolve) => (release = resolve))
    server.use(
      http.get(`${API}/public/shops/demo-salon/availability`, async ({ request }) => {
        await gate
        const q = new URL(request.url).searchParams
        return HttpResponse.json({
          timezone: 'Asia/Taipei',
          slots: standardSlots(q.get('from')!, q.get('to')!),
        })
      }),
    )
    renderApp(BOOK)
    const sunday = await screen.findByRole('button', { name: /10月11日/ })
    expect(sunday).not.toHaveAccessibleName(/沒有空檔/)
    expect(sunday).not.toHaveClass('day-off')
    expect(screen.getByText('查詢可預約時段…')).toBeInTheDocument()

    release()
    await screen.findByRole('button', { name: '10:00' })
    expect(screen.getByRole('button', { name: /10月11日.*沒有空檔/ })).toHaveClass('day-off')
  })

  it('時間依店家時區顯示:台北 14:00 = UTC 06:00', async () => {
    mockShop()
    renderApp(BOOK)
    expect(await screen.findByRole('button', { name: '14:00' })).toBeInTheDocument()
  })

  it('這一週沒空、之後才有 → 直接跳到最早有空的那週', async () => {
    const lateDay = addDays(today(), 9) // 第二週
    mockShop({
      slots: () => [
        { staff_id: STAFF_A.id, start: taipei(lateDay, '10:00'), end: taipei(lateDay, '11:00') },
      ],
    })
    renderApp(BOOK)
    const target = await screen.findByRole('button', { name: /10月14日/ })
    expect(target).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: '10:00' })).toBeInTheDocument()
  })

  it('整個視窗都沒空檔 → 提供「查看後面的日期」並抓下一個兩週', async () => {
    const calls: URLSearchParams[] = []
    mockShop({
      availabilityCalls: calls,
      slots: ({ from }) => (from === today() ? [] : standardSlots(from, addDays(from, 13))),
    })
    renderApp(BOOK)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '查看後面的日期' }))
    await screen.findByRole('button', { name: '10:00' })
    expect(calls.map((c) => c.get('from'))).toEqual([today(), addDays(today(), 14)])
    // 後端限制單次最多 14 天:每次查詢都不能超過
    for (const c of calls) {
      const days = (Date.parse(c.get('to')!) - Date.parse(c.get('from')!)) / 86_400_000 + 1
      expect(days).toBeLessThanOrEqual(14)
    }
  })

  it('完全沒有可預約時段 → 說明並引導聯絡店家', async () => {
    mockShop({ slots: () => [] })
    renderApp(BOOK)
    expect(await screen.findByText('這段期間沒有空檔。')).toBeInTheDocument()
  })

  it('下一步在選擇時間之前不可按,選了之後摘要與按鈕都更新', async () => {
    mockShop()
    renderApp(BOOK)
    const user = userEvent.setup()
    const next = await screen.findByRole('button', { name: '請先選擇時間' })
    expect(next).toBeDisabled()
    await user.click(await screen.findByRole('button', { name: '10:30' }))
    expect(screen.getByRole('button', { name: /下一步.*10:30/ })).toBeEnabled()
    expect(
      within(screen.getByRole('complementary')).getByText(/10月5日.*10:30/),
    ).toBeInTheDocument()
  })
})

describe('服務人員', () => {
  it('只有一位時不顯示選擇;多位時顯示,並依選擇查詢該員工的時段', async () => {
    const calls: URLSearchParams[] = []
    mockShop({ staff: [STAFF_A, STAFF_B], availabilityCalls: calls })
    renderApp(BOOK)
    const user = userEvent.setup()
    const b = await screen.findByRole('radio', { name: '陳小安' })
    expect(screen.getByRole('radio', { name: '不指定' })).toBeChecked()
    expect(calls[0].get('staff_id')).toBeNull()

    await user.click(b)
    await waitFor(() => expect(calls.at(-1)!.get('staff_id')).toBe(STAFF_B.id))
    expect(within(screen.getByRole('complementary')).getByText('陳小安')).toBeInTheDocument()
  })

  it('一位人員時不出現選擇器', async () => {
    mockShop({ staff: [STAFF_A] })
    renderApp(BOOK)
    await screen.findByRole('button', { name: '10:00' })
    expect(screen.queryByRole('radio')).not.toBeInTheDocument()
  })

  it('換人會清掉已選的時間(新的人那個時間不一定有空)', async () => {
    mockShop({ staff: [STAFF_A, STAFF_B] })
    renderApp(BOOK)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '10:30' }))
    await user.click(screen.getByRole('radio', { name: '陳小安' }))
    expect(await screen.findByRole('button', { name: '請先選擇時間' })).toBeDisabled()
  })
})

describe('送出預約申請', () => {
  async function toDetails(user: ReturnType<typeof userEvent.setup>) {
    await user.click(await screen.findByRole('button', { name: '10:30' }))
    await user.click(screen.getByRole('button', { name: /下一步/ }))
    await screen.findByRole('heading', { name: '填寫聯絡資料' })
  }

  it('空白送出 → 逐欄顯示錯誤、焦點移到第一個錯誤欄位、不會打 API', async () => {
    mockShop()
    let posted = false
    server.use(
      http.post(
        `${API}/public/shops/demo-salon/bookings`,
        () => ((posted = true), error(500, 'x')),
      ),
    )
    renderApp(BOOK)
    const user = userEvent.setup()
    await toDetails(user)
    await user.click(screen.getByRole('button', { name: '送出預約申請' }))

    expect(screen.getByText('請輸入姓名')).toBeInTheDocument()
    expect(screen.getByText('請輸入 Email')).toBeInTheDocument()
    expect(screen.getByLabelText(/姓名/)).toHaveFocus()
    expect(screen.getByLabelText(/姓名/)).toHaveAttribute('aria-invalid', 'true')
    expect(posted).toBe(false)

    await user.type(screen.getByLabelText(/姓名/), '王')
    expect(screen.queryByText('請輸入姓名')).not.toBeInTheDocument() // 開始輸入就清掉該欄的錯誤
  })

  it('Email 格式錯誤', async () => {
    mockShop()
    renderApp(BOOK)
    const user = userEvent.setup()
    await toDetails(user)
    await user.type(screen.getByLabelText(/姓名/), '王小明')
    await user.type(screen.getByLabelText(/Email/), 'not-an-email')
    await user.click(screen.getByRole('button', { name: '送出預約申請' }))
    expect(screen.getByText('Email 格式不正確')).toBeInTheDocument()
  })

  it('成功:送出的內容正確(去空白、電話空白則省略、不指定員工則不帶 staff_id),並說明預約尚未成立', async () => {
    mockShop()
    let body: unknown
    server.use(
      http.post(`${API}/public/shops/demo-salon/bookings`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(
          {
            id: 'b1',
            status: 'pending',
            starts_at: taipei('2026-10-05', '10:30'),
            ends_at: taipei('2026-10-05', '11:30'),
            message: 'x',
          },
          { status: 201 },
        )
      }),
    )
    renderApp(BOOK)
    const user = userEvent.setup()
    await toDetails(user)
    await user.type(screen.getByLabelText(/姓名/), '  王小明  ')
    await user.type(screen.getByLabelText(/Email/), ' ming@example.com ')
    await user.click(screen.getByRole('button', { name: '送出預約申請' }))

    expect(await screen.findByRole('heading', { name: '請到信箱確認預約' })).toBeInTheDocument()
    expect(body).toEqual({
      service_id: 'svc-cut',
      start: taipei('2026-10-05', '10:30'),
      customer: { name: '王小明', email: 'ming@example.com' },
    })
    expect(screen.getByText(/預約尚未成立/)).toBeInTheDocument()
    expect(screen.getByText('ming@example.com')).toBeInTheDocument()
    expect(screen.getByText(/24 小時內/)).toBeInTheDocument()
  })

  it('沒收到信:按「重新寄送」帶預約編號與 Email,顯示後端訊息並暫時停用按鈕', async () => {
    mockShop()
    let resent: unknown
    server.use(
      http.post(`${API}/public/shops/demo-salon/bookings`, () =>
        HttpResponse.json(
          {
            id: 'b-42',
            status: 'pending',
            starts_at: taipei('2026-10-05', '10:30'),
            ends_at: taipei('2026-10-05', '11:30'),
            message: 'x',
          },
          { status: 201 },
        ),
      ),
      http.post(`${API}/public/shops/demo-salon/bookings/b-42/resend`, async ({ request }) => {
        resent = await request.json()
        return HttpResponse.json({ message: '如果資料正確,確認信已重新寄出。' }, { status: 202 })
      }),
    )
    renderApp(BOOK)
    const user = userEvent.setup()
    await toDetails(user)
    await user.type(screen.getByLabelText(/姓名/), '王小明')
    await user.type(screen.getByLabelText(/Email/), ' Ming@Example.com ')
    await user.click(screen.getByRole('button', { name: '送出預約申請' }))

    await user.click(await screen.findByRole('button', { name: '沒收到?重新寄送確認信' }))
    expect(await screen.findByText('如果資料正確,確認信已重新寄出。')).toBeInTheDocument()
    expect(resent).toEqual({ email: 'Ming@Example.com' })
    expect(screen.getByRole('button', { name: '已重新寄出' })).toBeDisabled() // 避免連按
  })

  it('重寄被限流(429)→ 顯示原因', async () => {
    mockShop()
    server.use(
      http.post(`${API}/public/shops/demo-salon/bookings`, () =>
        HttpResponse.json(
          {
            id: 'b-42',
            status: 'pending',
            starts_at: taipei('2026-10-05', '10:30'),
            ends_at: taipei('2026-10-05', '11:30'),
            message: 'x',
          },
          { status: 201 },
        ),
      ),
      http.post(`${API}/public/shops/demo-salon/bookings/b-42/resend`, () =>
        error(429, '請求過於頻繁,請稍後再試'),
      ),
    )
    renderApp(BOOK)
    const user = userEvent.setup()
    await toDetails(user)
    await user.type(screen.getByLabelText(/姓名/), '王小明')
    await user.type(screen.getByLabelText(/Email/), 'ming@example.com')
    await user.click(screen.getByRole('button', { name: '送出預約申請' }))
    await user.click(await screen.findByRole('button', { name: '沒收到?重新寄送確認信' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('請求過於頻繁')
    expect(screen.getByRole('button', { name: '沒收到?重新寄送確認信' })).toBeEnabled()
  })

  it('指定員工與電話 → 一併送出', async () => {
    mockShop({ staff: [STAFF_A, STAFF_B] })
    let body: { staff_id?: string; customer: { phone?: string } } | undefined
    server.use(
      http.post(`${API}/public/shops/demo-salon/bookings`, async ({ request }) => {
        body = (await request.json()) as typeof body
        return HttpResponse.json(
          {
            id: 'b',
            status: 'pending',
            starts_at: taipei('2026-10-05', '10:30'),
            ends_at: '',
            message: '',
          },
          { status: 201 },
        )
      }),
    )
    renderApp(BOOK)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('radio', { name: '陳小安' }))
    await toDetails(user)
    await user.type(screen.getByLabelText(/姓名/), '王小明')
    await user.type(screen.getByLabelText(/Email/), 'a@example.com')
    await user.type(screen.getByLabelText(/手機/), '0912-345-678')
    await user.click(screen.getByRole('button', { name: '送出預約申請' }))
    await screen.findByRole('heading', { name: '請到信箱確認預約' })
    expect(body!.staff_id).toBe(STAFF_B.id)
    expect(body!.customer.phone).toBe('0912-345-678')
  })

  it('時段剛好被別人訂走(409)→ 顯示原因,並能回去重選且重新查詢時段', async () => {
    const calls: URLSearchParams[] = []
    mockShop({ availabilityCalls: calls })
    server.use(
      http.post(`${API}/public/shops/demo-salon/bookings`, () => error(409, '所選時段無法預約')),
    )
    renderApp(BOOK)
    const user = userEvent.setup()
    await toDetails(user)
    await user.type(screen.getByLabelText(/姓名/), '王小明')
    await user.type(screen.getByLabelText(/Email/), 'a@example.com')
    await user.click(screen.getByRole('button', { name: '送出預約申請' }))

    expect(await screen.findByRole('alert')).toHaveTextContent('所選時段無法預約')
    const before = calls.length
    await user.click(screen.getByRole('button', { name: '重新選擇時間' }))
    expect(await screen.findByRole('heading', { name: '選擇日期與時間' })).toBeInTheDocument()
    await waitFor(() => expect(calls.length).toBeGreaterThan(before)) // 重新取得最新時段
    expect(screen.getByRole('button', { name: '請先選擇時間' })).toBeDisabled() // 舊選擇已清掉
  })

  it('其他錯誤(429)→ 顯示訊息,保留已填的資料可再試', async () => {
    mockShop()
    server.use(
      http.post(`${API}/public/shops/demo-salon/bookings`, () =>
        error(429, '請求過於頻繁,請稍後再試'),
      ),
    )
    renderApp(BOOK)
    const user = userEvent.setup()
    await toDetails(user)
    await user.type(screen.getByLabelText(/姓名/), '王小明')
    await user.type(screen.getByLabelText(/Email/), 'a@example.com')
    await user.click(screen.getByRole('button', { name: '送出預約申請' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('請求過於頻繁')
    expect(screen.getByLabelText(/姓名/)).toHaveValue('王小明')
    expect(screen.getByRole('button', { name: '送出預約申請' })).toBeEnabled()
  })

  it('送出中不能重複點擊(避免重複預約)', async () => {
    mockShop()
    let count = 0
    server.use(
      http.post(`${API}/public/shops/demo-salon/bookings`, async () => {
        count++
        await new Promise((r) => setTimeout(r, 150))
        return HttpResponse.json(
          {
            id: 'b',
            status: 'pending',
            starts_at: taipei('2026-10-05', '10:30'),
            ends_at: '',
            message: '',
          },
          { status: 201 },
        )
      }),
    )
    renderApp(BOOK)
    const user = userEvent.setup()
    await toDetails(user)
    await user.type(screen.getByLabelText(/姓名/), '王小明')
    await user.type(screen.getByLabelText(/Email/), 'a@example.com')
    const submit = screen.getByRole('button', { name: '送出預約申請' })
    await user.click(submit)
    expect(screen.getByRole('button', { name: '送出中…' })).toBeDisabled()
    await user.click(screen.getByRole('button', { name: '送出中…' })).catch(() => {})
    await screen.findByRole('heading', { name: '請到信箱確認預約' })
    expect(count).toBe(1)
  })

  it('服務不存在 → 引導回服務列表', async () => {
    mockShop()
    renderApp('/s/demo-salon/book/ghost')
    expect(await screen.findByText('找不到這項服務')).toBeInTheDocument()
  })
})

describe('店家資訊', () => {
  it('顯示簡介、地址(地圖連結)與電話(撥號連結);電話連結只留數字與加號', async () => {
    mockShop({
      shop: {
        name: '森林系髮廊',
        timezone: 'Asia/Taipei',
        description: '專做自然捲。\n歡迎預約!',
        address: '台北市中山區南京東路一段 1 號',
        phone: '+886 2-1234-5678 #12',
      },
      services: [SERVICE],
    })
    renderApp('/s/demo-salon')
    expect(await screen.findByText(/專做自然捲/)).toBeInTheDocument()
    const address = screen.getByRole('link', { name: /南京東路/ })
    expect(address).toHaveAttribute(
      'href',
      `https://www.google.com/maps/search/?api=1&query=${encodeURIComponent('台北市中山區南京東路一段 1 號')}`,
    )
    expect(address).toHaveAttribute('rel', expect.stringContaining('noreferrer'))
    expect(screen.getByRole('link', { name: /2-1234-5678/ })).toHaveAttribute(
      'href',
      'tel:+886212345678;ext=12',
    )
  })

  it.each([
    ['02-1234-5678', 'tel:0212345678'],
    ['(02) 1234 5678', 'tel:0212345678'],
    ['+886 2-1234-5678', 'tel:+886212345678'],
    ['+886 2-1234-5678 #12', 'tel:+886212345678;ext=12'],
    ['02-1234-5678 # 7 8', 'tel:0212345678;ext=78'],
    ['02-1234-5678 #', 'tel:0212345678'],
  ])('電話 %s → %s(分機放在 ;ext=,不接在號碼後面)', (phone, href) => {
    expect(telHref(phone)).toBe(href)
  })

  it('沒填就什麼都不顯示', async () => {
    mockShop({ services: [SERVICE] })
    renderApp('/s/demo-salon')
    await screen.findByRole('link', { name: /剪髮/ })
    expect(screen.queryByLabelText('店家聯絡資訊')).not.toBeInTheDocument()
  })
})
