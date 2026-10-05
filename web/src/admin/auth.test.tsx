import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { HttpResponse, http } from 'msw'
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { freezeNow } from '../test/freeze'
import { renderApp } from '../test/render'
import { API, error, server } from '../test/server'
import { SHOP, USER, loginAs, mockShopMe, preloadAdmin } from '../test/admin'
import { safeReturnPath } from './paths'
import { clearSession, getSession, getStatus, restoreSession, setSession } from './session'

beforeAll(preloadAdmin)
beforeEach(() => freezeNow())

describe('safeReturnPath:登入後只回站內的後台 / 邀請頁(防開放式重新導向)', () => {
  it.each([
    ['/admin', '/admin'],
    ['/admin/demo-salon/bookings?view=pending', '/admin/demo-salon/bookings?view=pending'],
    ['/invitations/accept#token=abc', '/invitations/accept#token=abc'],
  ])('接受 %s', (input, expected) => expect(safeReturnPath(input)).toBe(expected))

  it.each([
    'https://evil.example.com/admin',
    '//evil.example.com/admin',
    '//admin',
    '/\\evil.example.com',
    'javascript:alert(1)',
    '/adminx',
    '/s/demo-salon',
    '',
    undefined,
    null,
    42,
  ])('一律退回 /admin:%j', (input) => expect(safeReturnPath(input)).toBe('/admin'))
})

describe('session:登入靠 HttpOnly cookie,前端不存 token', () => {
  it('登入後前端只記使用者是誰;localStorage 裡沒有 token 也沒有個資', () => {
    loginAs()
    expect(getSession()?.user.email).toBe(USER.email)
    expect(getStatus()).toBe('in')
    const stored = JSON.stringify({ ...localStorage })
    expect(stored).not.toContain(USER.email)
    expect(stored).not.toMatch(/token|eyJ/i)
    clearSession()
    expect(getSession()).toBeNull()
    expect(getStatus()).toBe('out')
  })

  it('登入 / 登出會通知其他分頁(訊號本身沒有任何機密);沒登入時清除不用通知', () => {
    const signal = () => localStorage.getItem('tenant-saas.auth-event')
    expect(signal()).toBeNull()
    setSession(USER)
    const first = signal()
    expect(first).toMatch(/^\d+$/)
    vi.setSystemTime(Date.now() + 1000)
    clearSession()
    expect(signal()).not.toBe(first)

    localStorage.clear()
    clearSession() // 本來就沒登入
    expect(signal()).toBeNull()
  })

  it('restoreSession:後端認得 cookie → 登入;401 或連不上 → 未登入', async () => {
    server.use(http.get(`${API}/auth/me`, () => HttpResponse.json(USER)))
    await restoreSession()
    expect(getSession()?.user).toEqual(USER)
    expect(getStatus()).toBe('in')

    server.use(http.get(`${API}/auth/me`, () => error(401, '未登入或憑證無效')))
    await restoreSession()
    expect(getStatus()).toBe('out')

    server.use(http.get(`${API}/auth/me`, () => HttpResponse.json(USER)))
    await restoreSession()
    server.use(http.get(`${API}/auth/me`, () => HttpResponse.error()))
    await restoreSession()
    expect(getStatus()).toBe('out')
  })

  it('啟動時的確認還沒回來,使用者就先登入了 → 不能被遲到的「未登入」結果蓋掉', async () => {
    let release: () => void = () => {}
    const gate = new Promise<void>((r) => (release = r))
    server.use(
      http.get(`${API}/auth/me`, async () => {
        await gate
        return error(401, '未登入或憑證無效')
      }),
    )
    const pending = restoreSession()
    setSession(USER) // 這時候使用者剛好按了登入
    release()
    await pending
    expect(getStatus()).toBe('in')
  })
})

describe('路由守衛', () => {
  it('未登入進後台 → 導向登入頁', async () => {
    renderApp('/admin/demo-salon/bookings')
    expect(await screen.findByRole('heading', { name: '登入' })).toBeInTheDocument()
  })

  it('登入成功 → 回到原本要去的頁面;token 不進 JS 可及之處(只靠 cookie)', async () => {
    const seen: (string | null)[] = []
    mockShopMe()
    server.use(
      http.post(`${API}/auth/login`, () =>
        HttpResponse.json({ token: 'aaa.bbb.ccc-secret', user: USER }),
      ),
      http.get(`${API}/t/demo-salon/bookings`, ({ request }) => {
        seen.push(request.headers.get('authorization'))
        return HttpResponse.json({ items: [], limit: 100, offset: 0 })
      }),
    )
    renderApp('/admin/demo-salon/bookings')
    const user = userEvent.setup()
    await user.type(await screen.findByLabelText('Email'), USER.email)
    await user.type(screen.getByLabelText('密碼'), 'password123')
    await user.click(screen.getByRole('button', { name: '登入' }))

    expect(await screen.findByRole('heading', { name: '預約' })).toBeInTheDocument()
    await waitFor(() => expect(seen.length).toBeGreaterThan(0))
    // 前端不自己帶 Authorization(那代表 JS 拿著 token);瀏覽器會自動帶 cookie
    expect(seen.every((h) => h === null)).toBe(true)
    expect(JSON.stringify({ ...localStorage, ...sessionStorage })).not.toContain('ccc-secret')
  })

  it('已登入的人打開登入頁 → 直接進後台', async () => {
    loginAs()
    server.use(http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])))
    renderApp('/admin/login')
    expect(await screen.findByRole('heading', { name: '我的店家' })).toBeInTheDocument()
  })
})

describe('登入表單', () => {
  it('帳號或密碼錯誤 → 只顯示一種訊息(後端也刻意不區分,避免洩漏哪些 Email 已註冊)', async () => {
    server.use(http.post(`${API}/auth/login`, () => error(401, '未登入或憑證無效')))
    renderApp('/admin/login')
    const user = userEvent.setup()
    await user.type(await screen.findByLabelText('Email'), 'a@example.com')
    await user.type(screen.getByLabelText('密碼'), 'wrong')
    await user.click(screen.getByRole('button', { name: '登入' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Email 或密碼錯誤。連續輸錯多次帳號會暫時鎖定',
    )
    expect(getSession()).toBeNull()
  })

  it('空白送出 → 不打 API、逐欄顯示錯誤', async () => {
    let called = false
    server.use(http.post(`${API}/auth/login`, () => ((called = true), error(500, 'x'))))
    renderApp('/admin/login')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '登入' }))
    expect(screen.getByText('請輸入 Email')).toBeInTheDocument()
    expect(screen.getByText('請輸入密碼')).toBeInTheDocument()
    expect(called).toBe(false)
  })

  it('被限流(429)→ 顯示後端的訊息', async () => {
    server.use(http.post(`${API}/auth/login`, () => error(429, '請求過於頻繁,請稍後再試')))
    renderApp('/admin/login')
    const user = userEvent.setup()
    await user.type(await screen.findByLabelText('Email'), 'a@example.com')
    await user.type(screen.getByLabelText('密碼'), 'whatever')
    await user.click(screen.getByRole('button', { name: '登入' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('請求過於頻繁')
  })
})

describe('註冊表單', () => {
  it('密碼太短 → 前端先擋,不送出', async () => {
    let called = false
    server.use(http.post(`${API}/auth/register`, () => ((called = true), error(500, 'x'))))
    renderApp('/admin/register')
    const user = userEvent.setup()
    await user.type(await screen.findByLabelText('姓名'), '王小明')
    await user.type(screen.getByLabelText('Email'), 'a@example.com')
    await user.type(screen.getByLabelText('密碼'), 'short')
    await user.click(screen.getByRole('button', { name: '建立帳號' }))
    expect(screen.getByText('密碼至少 8 個字')).toBeInTheDocument()
    expect(called).toBe(false)
  })

  it('Email 已被註冊(409)→ 顯示後端訊息', async () => {
    server.use(http.post(`${API}/auth/register`, () => error(409, '此 Email 已註冊')))
    renderApp('/admin/register')
    const user = userEvent.setup()
    await user.type(await screen.findByLabelText('姓名'), '王小明')
    await user.type(screen.getByLabelText('Email'), 'a@example.com')
    await user.type(screen.getByLabelText('密碼'), 'password123')
    await user.click(screen.getByRole('button', { name: '建立帳號' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('此 Email 已註冊')
  })
})

describe('登入過期', () => {
  it('後端回 401 → 清掉登入狀態並回到登入頁', async () => {
    loginAs()
    mockShopMe()
    server.use(http.get(`${API}/t/demo-salon/bookings`, () => error(401, '未登入或憑證無效')))
    const { client } = renderApp('/admin/demo-salon/bookings')
    expect(await screen.findByRole('heading', { name: '登入' })).toBeInTheDocument()
    expect(getSession()).toBeNull()
    // 不是按登出鈕,而是登入狀態自己消失:快取一樣要清掉
    await waitFor(() =>
      expect(
        client.getQueryCache().find({ queryKey: ['admin', 'shop', 'demo-salon'] }),
      ).toBeUndefined(),
    )
  })

  it('另一個分頁登出 → 這個分頁向後端確認後也跟著登出', async () => {
    loginAs()
    server.use(http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])))
    const { client } = renderApp('/admin')
    expect(await screen.findByRole('heading', { name: '我的店家' })).toBeInTheDocument()
    expect(client.getQueryCache().getAll().length).toBeGreaterThan(0)

    // 那個分頁登出後 cookie 已被清掉,後端不再認得
    server.use(http.get(`${API}/auth/me`, () => error(401, '未登入或憑證無效')))
    window.dispatchEvent(new StorageEvent('storage', { key: 'tenant-saas.auth-event' }))
    expect(await screen.findByRole('heading', { name: '登入' })).toBeInTheDocument()
    await waitFor(() => expect(client.getQueryCache().getAll()).toHaveLength(0))
  })

  it('另一個分頁登入 → 這個分頁(原本未登入)也進得去', async () => {
    server.use(
      http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])),
      http.get(`${API}/auth/me`, () => HttpResponse.json(USER)),
    )
    renderApp('/admin/login')
    expect(await screen.findByRole('heading', { name: '登入' })).toBeInTheDocument()
    window.dispatchEvent(new StorageEvent('storage', { key: 'tenant-saas.auth-event' }))
    expect(await screen.findByRole('heading', { name: '我的店家' })).toBeInTheDocument()
  })

  it('不相干的 storage 事件不會觸發向後端確認', async () => {
    loginAs()
    let asked = 0
    server.use(
      http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])),
      http.get(`${API}/auth/me`, () => ((asked += 1), HttpResponse.json(USER))),
    )
    renderApp('/admin')
    await screen.findByRole('heading', { name: '我的店家' })
    window.dispatchEvent(new StorageEvent('storage', { key: 'something-else' }))
    await new Promise((r) => setTimeout(r, 20))
    expect(asked).toBe(0)
  })

  it('登出會清掉所有快取,下一位登入的人看不到上一位的資料', async () => {
    loginAs()
    server.use(http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])))
    const { client } = renderApp('/admin')
    await screen.findByRole('heading', { name: '我的店家' })
    expect(client.getQueryCache().getAll().length).toBeGreaterThan(0)

    let loggedOut = 0
    server.use(
      http.post(
        `${API}/auth/logout`,
        () => ((loggedOut += 1), new HttpResponse(null, { status: 204 })),
      ),
    )
    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: '登出' }))
    expect(await screen.findByRole('heading', { name: '登入' })).toBeInTheDocument()
    expect(client.getQueryCache().getAll()).toHaveLength(0)
    // HttpOnly 的 cookie 前端刪不掉,一定要請後端清
    expect(loggedOut).toBe(1)
  })

  it('登出時後端連不上 → 仍然在前端登出(盡力而為)', async () => {
    loginAs()
    server.use(
      http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])),
      http.post(`${API}/auth/logout`, () => HttpResponse.error()),
    )
    renderApp('/admin')
    await screen.findByRole('heading', { name: '我的店家' })
    await userEvent.setup().click(screen.getByRole('button', { name: '登出' }))
    expect(await screen.findByRole('heading', { name: '登入' })).toBeInTheDocument()
    expect(getSession()).toBeNull()
  })
})

describe('忘記密碼 / 重設密碼', () => {
  it('登入頁有「忘記密碼?」連結;送出 Email 後顯示後端的統一訊息(不洩漏是否註冊)', async () => {
    let sent: unknown
    server.use(
      http.post(`${API}/auth/forgot-password`, async ({ request }) => {
        sent = await request.json()
        return HttpResponse.json(
          { message: '如果這個 Email 已註冊,重設密碼的信已經寄出。' },
          { status: 202 },
        )
      }),
    )
    renderApp('/admin/login')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('link', { name: '忘記密碼?' }))
    await user.click(await screen.findByRole('button', { name: '寄送重設信' }))
    expect(await screen.findByText('請輸入 Email')).toBeInTheDocument() // 先在前端擋下
    expect(sent).toBeUndefined()

    await user.type(screen.getByLabelText('Email'), '  a@example.com ')
    await user.click(screen.getByRole('button', { name: '寄送重設信' }))
    expect(await screen.findByText(/重設密碼的信已經寄出/)).toBeInTheDocument()
    expect(sent).toEqual({ email: 'a@example.com' })
  })

  it('重設:token 從 # 讀取;兩次密碼要一致;成功後導向登入', async () => {
    let body: unknown
    server.use(
      http.post(`${API}/auth/reset-password`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ message: '密碼已更新,請用新密碼登入。' })
      }),
    )
    const token = 'a'.repeat(64)
    renderApp(`/admin/reset#token=${token}`)
    const user = userEvent.setup()
    await user.type(await screen.findByLabelText('新密碼'), 'new-password-2')
    await user.type(screen.getByLabelText('再輸入一次'), 'different-pass')
    await user.click(screen.getByRole('button', { name: '更新密碼' }))
    expect(await screen.findByText('兩次輸入的密碼不一致')).toBeInTheDocument()
    expect(body).toBeUndefined()

    await user.clear(screen.getByLabelText('再輸入一次'))
    await user.type(screen.getByLabelText('再輸入一次'), 'new-password-2')
    await user.click(screen.getByRole('button', { name: '更新密碼' }))
    expect(await screen.findByText('密碼已更新,請用新密碼登入。')).toBeInTheDocument()
    expect(body).toEqual({ token, password: 'new-password-2' })
    expect(screen.getByRole('link', { name: '前往登入' })).toHaveAttribute('href', '/admin/login')
  })

  it('連結缺少 token → 顯示無效;token 過期 → 顯示後端訊息與重新申請連結', async () => {
    const first = renderApp('/admin/reset')
    expect(await screen.findByText(/重設連結不完整/)).toBeInTheDocument()
    first.unmount()

    server.use(
      http.post(`${API}/auth/reset-password`, () => error(400, '重設連結無效或已過期,請重新申請')),
    )
    renderApp(`/admin/reset#token=${'b'.repeat(64)}`)
    const user = userEvent.setup()
    await user.type(await screen.findByLabelText('新密碼'), 'new-password-2')
    await user.type(screen.getByLabelText('再輸入一次'), 'new-password-2')
    await user.click(screen.getByRole('button', { name: '更新密碼' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('已過期')
    expect(screen.getByRole('link', { name: '重新申請' })).toHaveAttribute('href', '/admin/forgot')
  })
})
