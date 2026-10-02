import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { HttpResponse, http } from 'msw'
import { beforeAll, beforeEach, describe, expect, it } from 'vitest'
import { freezeNow } from '../test/freeze'
import { renderApp } from '../test/render'
import { API, error, server } from '../test/server'
import { SHOP, USER, fakeJwt, loginAs, mockShopMe, preloadAdmin } from '../test/admin'
import { safeReturnPath } from './paths'
import { clearSession, getSession, jwtExpiry, setSession } from './session'

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

describe('session', () => {
  it('jwtExpiry 解出 exp(毫秒);壞掉的 token 回 null', () => {
    const token = fakeJwt(600)
    expect(jwtExpiry(token)).toBeGreaterThan(Date.now())
    expect(jwtExpiry('garbage')).toBeNull()
    expect(jwtExpiry('a.b.c')).toBeNull()
    expect(jwtExpiry(`${btoa('{}')}.${btoa('{"exp":"soon"}')}.x`)).toBeNull()
  })

  it('沒有 exp 的 token 不接受登入(無法決定何時該過期)', () => {
    expect(setSession('a.b.c', USER)).toBeNull()
    expect(getSession()).toBeNull()
  })

  it('登入狀態存進 localStorage,清除後消失', () => {
    loginAs()
    expect(localStorage.getItem('tenant-saas.session')).toContain(USER.email)
    clearSession()
    expect(localStorage.getItem('tenant-saas.session')).toBeNull()
    expect(getSession()).toBeNull()
  })
})

describe('路由守衛', () => {
  it('未登入進後台 → 導向登入頁', async () => {
    renderApp('/admin/demo-salon/bookings')
    expect(await screen.findByRole('heading', { name: '登入' })).toBeInTheDocument()
  })

  it('登入成功 → 回到原本要去的頁面,而且 token 帶在後續請求上', async () => {
    const seen: (string | null)[] = []
    mockShopMe()
    server.use(
      http.post(`${API}/auth/login`, () => HttpResponse.json({ token: fakeJwt(), user: USER })),
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
    expect(seen[0]).toMatch(/^Bearer .+\..+\..+$/)
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
    expect(await screen.findByRole('alert')).toHaveTextContent('Email 或密碼錯誤')
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
    expect(localStorage.getItem('tenant-saas.session')).toBeNull()
    // 不是按登出鈕,而是登入狀態自己消失:快取一樣要清掉
    await waitFor(() =>
      expect(
        client.getQueryCache().find({ queryKey: ['admin', 'shop', 'demo-salon'] }),
      ).toBeUndefined(),
    )
  })

  it('localStorage 裡過期的登入狀態會被丟掉,不當成已登入', () => {
    localStorage.setItem(
      'tenant-saas.session',
      JSON.stringify({ token: fakeJwt(-10), expiresAt: Date.now() - 1000, user: USER }),
    )
    // 模組載入時就讀過一次;這裡驗證 storage 事件重新讀取時的行為
    window.dispatchEvent(new StorageEvent('storage', { key: 'tenant-saas.session' }))
    expect(getSession()).toBeNull()
    expect(localStorage.getItem('tenant-saas.session')).toBeNull()
  })

  it('另一個分頁登出 → 這個分頁也跟著登出', async () => {
    loginAs()
    server.use(http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])))
    const { client } = renderApp('/admin')
    expect(await screen.findByRole('heading', { name: '我的店家' })).toBeInTheDocument()
    expect(client.getQueryCache().getAll().length).toBeGreaterThan(0)

    localStorage.removeItem('tenant-saas.session')
    window.dispatchEvent(new StorageEvent('storage', { key: 'tenant-saas.session' }))
    expect(await screen.findByRole('heading', { name: '登入' })).toBeInTheDocument()
    await waitFor(() => expect(client.getQueryCache().getAll()).toHaveLength(0))
  })

  it('登出會清掉所有快取,下一位登入的人看不到上一位的資料', async () => {
    loginAs()
    server.use(http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])))
    const { client } = renderApp('/admin')
    await screen.findByRole('heading', { name: '我的店家' })
    expect(client.getQueryCache().getAll().length).toBeGreaterThan(0)

    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: '登出' }))
    expect(await screen.findByRole('heading', { name: '登入' })).toBeInTheDocument()
    expect(client.getQueryCache().getAll()).toHaveLength(0)
  })
})
