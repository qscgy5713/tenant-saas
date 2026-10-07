import { screen, waitFor, within } from '@testing-library/react'
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

describe('Email 驗證', () => {
  it('點信中的連結:token 從 # 讀取,送出一次,成功顯示後端訊息', async () => {
    const calls: unknown[] = []
    server.use(
      http.post(`${API}/auth/verify-email`, async ({ request }) => {
        calls.push(await request.json())
        return HttpResponse.json({ message: 'Email 已驗證,現在可以建立店家了。' })
      }),
    )
    const token = 'c'.repeat(64)
    renderApp(`/admin/verify#token=${token}`)
    expect(await screen.findByText('Email 已驗證,現在可以建立店家了。')).toBeInTheDocument()
    expect(calls).toEqual([{ token }]) // 只送一次:連結只能用一次
    expect(screen.getByRole('link', { name: '前往後台' })).toHaveAttribute('href', '/admin')
  })

  it('開發模式(StrictMode,effect 跑兩次)下也只送一次:連結只能用一次,第二次會變成「已使用」', async () => {
    let called = 0
    server.use(
      http.post(`${API}/auth/verify-email`, () => {
        called += 1
        return called === 1
          ? HttpResponse.json({ message: 'Email 已驗證,現在可以建立店家了。' })
          : error(400, '驗證連結無效或已過期,請登入後重新寄送')
      }),
    )
    renderApp(`/admin/verify#token=${'e'.repeat(64)}`, { strict: true })
    expect(await screen.findByText('Email 已驗證,現在可以建立店家了。')).toBeInTheDocument()
    expect(called).toBe(1)
  })

  it('連結缺 token → 不送出、顯示無效;連結過期 → 顯示後端訊息', async () => {
    let called = 0
    server.use(
      http.post(`${API}/auth/verify-email`, () => {
        called += 1
        return error(400, '驗證連結無效或已過期,請登入後重新寄送')
      }),
    )
    const first = renderApp('/admin/verify')
    expect(await screen.findByText(/驗證連結不完整/)).toBeInTheDocument()
    expect(called).toBe(0)
    first.unmount()

    renderApp(`/admin/verify#token=${'d'.repeat(64)}`)
    expect(await screen.findByRole('alert')).toHaveTextContent('已過期')
  })

  it('未驗證:店家列表顯示提醒,可重寄驗證信;已驗證不顯示', async () => {
    loginAs({ ...USER, email_verified: false })
    let resent = 0
    server.use(
      http.get(`${API}/tenants`, () => HttpResponse.json([])),
      http.post(`${API}/auth/resend-verification`, () => {
        resent += 1
        return HttpResponse.json({ message: '驗證信已寄出,請查看信箱。' }, { status: 202 })
      }),
    )
    const first = renderApp('/admin')
    const user = userEvent.setup()
    expect(await screen.findByText(/請驗證你的 Email/)).toBeInTheDocument()
    expect(screen.getByText(/owner@demo.example.com/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '重新寄送驗證信' }))
    expect(await screen.findByText('驗證信已寄出,請查看信箱。')).toBeInTheDocument()
    expect(resent).toBe(1)
    first.unmount()

    loginAs(USER)
    renderApp('/admin')
    await screen.findByRole('heading', { name: '我的店家' })
    expect(screen.queryByText(/請驗證你的 Email/)).not.toBeInTheDocument()
  })

  it('重寄太頻繁(429)→ 顯示稍後再試', async () => {
    loginAs({ ...USER, email_verified: false })
    server.use(
      http.get(`${API}/tenants`, () => HttpResponse.json([])),
      http.post(`${API}/auth/resend-verification`, () => error(429, '請求過於頻繁,請稍後再試')),
    )
    renderApp('/admin')
    await userEvent.setup().click(await screen.findByRole('button', { name: '重新寄送驗證信' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('寄了太多封')
  })

  it('「我已驗證,重新檢查」:向後端重新確認,提醒消失', async () => {
    loginAs({ ...USER, email_verified: false })
    server.use(
      http.get(`${API}/tenants`, () => HttpResponse.json([])),
      http.get(`${API}/auth/me`, () => HttpResponse.json({ ...USER, email_verified: true })),
    )
    renderApp('/admin')
    await userEvent.setup().click(await screen.findByRole('button', { name: '我已驗證,重新檢查' }))
    await waitFor(() => expect(screen.queryByText(/請驗證你的 Email/)).not.toBeInTheDocument())
  })
})

describe('登出所有裝置', () => {
  it('確認後呼叫後端,成功才清除登入並回到登入頁', async () => {
    loginAs()
    let called = 0
    server.use(
      http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])),
      http.post(`${API}/auth/logout-all`, () => {
        called += 1
        return new HttpResponse(null, { status: 204 })
      }),
    )
    renderApp('/admin')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '登出所有裝置' }))
    const dialog = await screen.findByRole('dialog')
    expect(called).toBe(0) // 還沒確認
    await user.click(within(dialog).getByRole('button', { name: '登出所有裝置' }))
    expect(await screen.findByRole('heading', { name: '登入' })).toBeInTheDocument()
    expect(called).toBe(1)
    expect(getSession()).toBeNull()
  })

  it('後端失敗時不能假裝成功:仍在登入狀態,視窗顯示錯誤', async () => {
    loginAs()
    server.use(
      http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])),
      http.post(`${API}/auth/logout-all`, () => HttpResponse.error()),
    )
    renderApp('/admin')
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '登出所有裝置' }))
    const dialog = await screen.findByRole('dialog')
    await user.click(within(dialog).getByRole('button', { name: '登出所有裝置' }))
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('尚未登出')
    expect(getSession()).not.toBeNull()
  })
})

describe('刪除帳號', () => {
  const open = async () => {
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '刪除我的帳號' }))
    return { user, dialog: await screen.findByRole('dialog') }
  }

  it('輸入密碼確認後送出;成功才清除登入並回到登入頁', async () => {
    loginAs()
    const bodies: unknown[] = []
    server.use(
      http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])),
      http.post(`${API}/auth/delete-account`, async ({ request }) => {
        bodies.push(await request.json())
        return new HttpResponse(null, { status: 204 })
      }),
    )
    renderApp('/admin')
    const { user, dialog } = await open()
    expect(bodies).toHaveLength(0) // 還沒確認
    await user.type(within(dialog).getByLabelText('輸入密碼確認'), 'my-password-1')
    await user.click(within(dialog).getByRole('button', { name: '永久刪除帳號' }))
    expect(await screen.findByRole('heading', { name: '登入' })).toBeInTheDocument()
    expect(bodies).toEqual([{ password: 'my-password-1' }])
    expect(getSession()).toBeNull()
  })

  it('後端拒絕(密碼不對 / 還是店主)→ 原因顯示在視窗裡,仍在登入狀態', async () => {
    loginAs()
    server.use(
      http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])),
      http.post(`${API}/auth/delete-account`, () =>
        error(409, '你還是某家店的擁有者,請先刪除那家店(設定頁 → 刪除店家)再刪除帳號'),
      ),
    )
    renderApp('/admin')
    const { user, dialog } = await open()
    await user.type(within(dialog).getByLabelText('輸入密碼確認'), 'x'.repeat(8))
    await user.click(within(dialog).getByRole('button', { name: '永久刪除帳號' }))
    expect(await within(dialog).findByRole('alert')).toHaveTextContent('還是某家店的擁有者')
    expect(getSession()).not.toBeNull()
    expect(dialog).toHaveAttribute('open')
  })

  it('關閉視窗會清掉已輸入的密碼', async () => {
    loginAs()
    server.use(http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])))
    renderApp('/admin')
    const { user, dialog } = await open()
    await user.type(within(dialog).getByLabelText('輸入密碼確認'), 'secret-pass-1')
    await user.click(within(dialog).getByRole('button', { name: '取消' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    await user.click(screen.getByRole('button', { name: '刪除我的帳號' }))
    expect(within(await screen.findByRole('dialog')).getByLabelText('輸入密碼確認')).toHaveValue('')
  })
})

describe('最近的帳號活動', () => {
  it('展開後列出事件(最新在前);沒有紀錄時說明', async () => {
    loginAs()
    server.use(
      http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])),
      http.get(`${API}/auth/security-events`, () =>
        HttpResponse.json([
          { kind: 'locked', created_at: '2026-10-06T02:00:00Z' },
          { kind: 'login_failed', created_at: '2026-10-06T01:59:00Z' },
          { kind: 'login', created_at: '2026-10-06T01:00:00Z' },
        ]),
      ),
    )
    renderApp('/admin')
    const user = userEvent.setup()
    await user.click(await screen.findByText('最近的帳號活動'))
    const items = await screen.findAllByText(/鎖定|登入失敗|^登入$/)
    expect(items.map((i) => i.textContent)).toEqual([
      '連續失敗,帳號暫時鎖定',
      '登入失敗(密碼錯誤)',
      '登入',
    ])
    expect(screen.getByText(/不是你做的登入/)).toBeInTheDocument()
  })

  it('沒有紀錄', async () => {
    loginAs()
    server.use(http.get(`${API}/tenants`, () => HttpResponse.json([SHOP()])))
    renderApp('/admin')
    await userEvent.setup().click(await screen.findByText('最近的帳號活動'))
    expect(await screen.findByText('沒有紀錄。')).toBeInTheDocument()
  })
})
