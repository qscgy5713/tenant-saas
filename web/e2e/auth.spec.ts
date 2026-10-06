import { expect, test } from '@playwright/test'
import {
  API,
  PASSWORD,
  anonymousApi,
  createShop,
  linkIn,
  loginViaUi,
  signIn,
  uniq,
  waitForMail,
} from './helpers'

test('註冊 → 開店 → 登入憑證是 HttpOnly cookie → 重新整理仍登入 → 登出', async ({
  page,
  context,
}) => {
  const email = `reg-${uniq()}@example.com`
  const slug = `e2e-${uniq()}`

  await page.goto('/admin/register')
  await page.getByLabel('姓名').fill('新店主')
  await page.getByLabel('Email').fill(email)
  await page.getByLabel('密碼').fill(PASSWORD)
  await page.getByRole('button', { name: '建立帳號' }).click()
  await expect(page.getByRole('heading', { name: '我的店家' })).toBeVisible()

  // 沒驗證 Email 不能開店:提醒 + 後端也擋(錯誤訊息說明怎麼辦)
  await expect(page.getByText(/請驗證你的 Email/)).toBeVisible()
  await page.getByRole('button', { name: '建立店家' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('店家名稱').fill('我的第一家店')
  await dialog.getByLabel(/預約頁網址代稱/).fill(slug)
  await dialog.getByRole('button', { name: '建立店家' }).click()
  await expect(dialog.getByRole('alert')).toContainText('請先驗證 Email')
  await dialog.getByRole('button', { name: '取消' }).click()

  // 從信箱點驗證連結(在另一個分頁開,和真實使用一樣),回來按「重新檢查」
  const mail = await waitForMail(context.request, email, '驗證')
  const verifyPage = await context.newPage()
  await verifyPage.goto(linkIn(mail, '/admin/verify'))
  await expect(verifyPage.getByRole('heading', { name: 'Email 已驗證' })).toBeVisible()
  await verifyPage.close()
  await page.getByRole('button', { name: '我已驗證,重新檢查' }).click()
  await expect(page.getByText(/請驗證你的 Email/)).toHaveCount(0)

  await page.getByRole('button', { name: '建立店家' }).click()
  await dialog.getByLabel('店家名稱').fill('我的第一家店')
  await dialog.getByLabel(/預約頁網址代稱/).fill(slug)
  await dialog.getByRole('button', { name: '建立店家' }).click()
  await expect(page).toHaveURL(new RegExp(`/admin/${slug}`))

  // 憑證在 HttpOnly cookie 裡:頁面的 JavaScript 讀不到,localStorage 也沒有 token
  expect(await page.evaluate(() => document.cookie)).toBe('')
  expect(await page.evaluate(() => JSON.stringify({ ...localStorage }))).not.toMatch(/eyJ|token/i)
  const session = (await context.cookies()).find((c) => c.name === 'session')
  expect(session).toMatchObject({ httpOnly: true, sameSite: 'Strict', path: '/' })

  // 重新整理仍在登入狀態(靠 GET /auth/me 確認)
  await page.reload()
  await expect(page).toHaveURL(new RegExp(`/admin/${slug}`))
  await expect(page.getByText('我的第一家店').first()).toBeVisible()

  // 登出:cookie 被後端清掉,重新整理後回到登入頁
  await page.getByRole('button', { name: '登出' }).click()
  await expect(page.getByRole('heading', { name: '登入' })).toBeVisible()
  expect((await context.cookies()).find((c) => c.name === 'session')).toBeUndefined()
  await page.goto(`/admin/${slug}`)
  await expect(page.getByRole('heading', { name: '登入' })).toBeVisible()

  // 重新登入 —— 會回到登出前想去的那家店(登入頁記得原本要去的頁面)
  await signIn(page, email)
  await expect(page).toHaveURL(new RegExp(`/admin/${slug}`))
  await expect(page.getByText('我的第一家店').first()).toBeVisible()
})

test('密碼錯誤與帳號不存在顯示同一種訊息(不洩漏哪些 Email 已註冊)', async ({ page, request }) => {
  const shop = await createShop(request)
  const message = async (email: string) => {
    await loginViaUi(page, email, 'wrong-password-1')
    return page.getByRole('alert').first().innerText()
  }
  const known = await message(shop.owner.email)
  const unknown = await message(`nobody-${uniq()}@example.com`)
  expect(known).toContain('Email 或密碼錯誤')
  expect(unknown).toBe(known)
})

test('CSRF:帶著登入 cookie 的寫入請求,Origin 不對就被擋(走完整條 Vite 代理 → 後端)', async ({
  page,
  context,
  request,
}) => {
  const shop = await createShop(request)
  await signIn(page, shop.owner.email)

  const body = { slug: `csrf-${uniq()}`, name: '不該被建立的店', timezone: 'Asia/Taipei' }
  const evil = await context.request.post('/api/tenants', {
    data: body,
    headers: { Origin: 'http://evil.example' },
  })
  expect(evil.status()).toBe(403)
  const missing = await context.request.post('/api/tenants', { data: body })
  // Playwright 的 API 請求預設不帶 Origin → 同樣被拒
  expect(missing.status()).toBe(403)

  const ok = await context.request.post('/api/tenants', {
    data: body,
    headers: { Origin: 'http://localhost:5273' },
  })
  expect(ok.status()).toBe(201)

  // 讀取不受影響
  expect((await context.request.get('/api/auth/me')).status()).toBe(200)
  // 沒有 cookie 的請求仍然是 401
  const anonymous = await anonymousApi()
  expect((await anonymous.get(`${API}/auth/me`)).status()).toBe(401)
  await anonymous.dispose()
})

test('登出所有裝置:另一個瀏覽器的登入也立刻失效', async ({ page, browser, request }) => {
  const shop = await createShop(request)

  await signIn(page, shop.owner.email)
  await page.goto('/admin')
  await expect(page.getByRole('heading', { name: '我的店家' })).toBeVisible()

  const otherContext = await browser.newContext()
  const other = await otherContext.newPage()
  await signIn(other, shop.owner.email)
  await other.goto('/admin')
  await expect(other.getByRole('heading', { name: '我的店家' })).toBeVisible()

  // 簽發時間只到秒,撤銷「同一秒內」簽發的 token 也算失效,所以不必等
  await page.getByRole('button', { name: '登出所有裝置' }).click()
  await page.getByRole('dialog').getByRole('button', { name: '登出所有裝置' }).click()
  await expect(page.getByRole('heading', { name: '登入' })).toBeVisible()

  // 另一個瀏覽器重新整理後被登出
  await other.reload()
  await expect(other.getByRole('heading', { name: '登入' })).toBeVisible()
  await otherContext.close()

  // 重新登入可以正常使用(等到下一秒,避開撤銷同一秒的窗口)
  await page.waitForTimeout(1100)
  await signIn(page, shop.owner.email)
  await page.goto('/admin')
  await expect(page.getByRole('heading', { name: '我的店家' })).toBeVisible()
})

test('未登入的瀏覽器(例如在手機上開信)點驗證連結:成功畫面不會被導走', async ({
  request,
  browser,
}) => {
  const email = `anon-${uniq()}@example.com`
  const r = await request.post(`${API}/auth/register`, {
    data: { email, password: PASSWORD, name: '路人' },
  })
  expect(r.ok()).toBeTruthy()
  const mail = await waitForMail(request, email, '驗證')

  const context = await browser.newContext() // 沒有任何 cookie
  const page = await context.newPage()
  await page.goto(linkIn(mail, '/admin/verify'))
  await expect(page.getByRole('heading', { name: 'Email 已驗證' })).toBeVisible()
  // 驗證成功後頁面會向後端確認登入狀態(沒登入會得到 401),不能因此被導去登入頁
  await page.waitForTimeout(2000)
  await expect(page).toHaveURL(/\/admin\/verify/)
  await expect(page.getByRole('heading', { name: 'Email 已驗證' })).toBeVisible()
  await context.close()
})
