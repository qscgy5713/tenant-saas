import { expect, test } from '@playwright/test'
import {
  addStaff,
  bookAsOwner,
  createShop,
  makeBookable,
  signIn,
  taipeiAt,
  taipeiDay,
  uniq,
  waitForMail,
} from './helpers'

test('顧客用管理連結自己改期:選新時段 → 改期成功,預約真的換了時間', async ({ page, request }) => {
  const shop = await createShop(request)
  const customerEmail = `cust-${uniq()}@example.com`
  const created = await bookAsOwner(request, shop, taipeiAt(taipeiDay(3), '10:00'), {
    name: '王小明',
    email: customerEmail,
  })
  await page.goto(`/bookings/${created.manage_token}`)
  await expect(page.getByRole('button', { name: '改期' })).toBeVisible()
  // 改期前頁面上顯示的預約時間
  const timeText = async () => (await page.locator('dl.facts').innerText()).trim()
  const before = await timeText()
  await page.getByRole('button', { name: '改期' }).click()

  const section = page.getByRole('region', { name: '改期' })
  await expect(section.getByText(/服務人員維持不變/)).toBeVisible()
  // 挑一個和目前不同的時段(目前的時段被預約自己占著,不會出現在可選清單裡)
  const slot = section.locator('button.slot').first()
  await expect(slot).toBeVisible()
  await slot.click()
  await section.getByRole('button', { name: /^改到 / }).click()
  await expect(page.getByText('已改期。')).toBeVisible()

  // 重新整理後顯示的是新時間,不是原本的(從伺服器重新讀,不是畫面上的暫時狀態)
  await page.reload()
  await expect(page.getByRole('button', { name: '改期' })).toBeVisible()
  expect(await timeText()).not.toBe(before)
})

test('員工(非店主)的視角:看不到店主與管理者的功能,直接開網址也進不去', async ({
  page,
  request,
}) => {
  const shop = await createShop(request)
  const staff = await addStaff(request, shop, '員工小安')
  await makeBookable(request, shop, staff, shop.serviceId)

  await signIn(page, staff.email)
  await page.goto(`/admin/${shop.slug}`)
  const nav = page.getByRole('navigation', { name: '後台選單' })
  await expect(nav.getByRole('link', { name: '預約' })).toBeVisible()
  for (const hidden of ['設定', '稽核', '顧客', '方案']) {
    await expect(nav.getByRole('link', { name: hidden })).toHaveCount(0)
  }
  // 只有店主的操作:直接開網址是「沒有權限」,不是壞掉
  await page.goto(`/admin/${shop.slug}/settings`)
  await expect(page.getByRole('heading', { name: '沒有權限' })).toBeVisible()
  // 團隊頁:看得到成員,但沒有邀請、移交店主、停用別人的按鈕
  await page.goto(`/admin/${shop.slug}/team`)
  await expect(page.locator('li.member-row', { hasText: shop.owner.email })).toBeVisible()
  await expect(page.getByRole('button', { name: '邀請成員' })).toHaveCount(0)
  await expect(page.getByRole('button', { name: '移交店主' })).toHaveCount(0)
})

test('店主移交店主身分:對方收到信、角色對調、舊店主立刻失去店主權限', async ({
  page,
  browser,
  request,
}) => {
  const shop = await createShop(request)
  const manager = await addStaff(request, shop, '新店主')
  await signIn(page, shop.owner.email)
  await page.goto(`/admin/${shop.slug}/team`)
  const row = page.locator('li.member-row', { hasText: manager.email })
  await row.getByRole('button', { name: '移交店主' }).click()
  const dialog = page.getByRole('dialog')
  // 密碼不對:原因顯示在視窗裡
  await dialog.getByLabel('輸入你的密碼確認').fill('wrong-password-x')
  await dialog.getByRole('button', { name: '移交' }).click()
  await expect(dialog.getByRole('alert')).toContainText('密碼不正確')
  await dialog.getByLabel('輸入你的密碼確認').fill('password-e2e-1')
  await dialog.getByRole('button', { name: '移交' }).click()

  const mail = await waitForMail(request, manager.email, '成為')
  expect(mail.text).toContain(shop.name)
  // 舊店主自己:不再是店主,「設定」消失
  await page.goto(`/admin/${shop.slug}`)
  const nav = page.getByRole('navigation', { name: '後台選單' })
  await expect(nav.getByRole('link', { name: '設定' })).toHaveCount(0)
  // 新店主:看得到「設定」
  const ctx = await browser.newContext()
  const other = await ctx.newPage()
  await signIn(other, manager.email)
  await other.goto(`/admin/${shop.slug}`)
  await expect(
    other.getByRole('navigation', { name: '後台選單' }).getByRole('link', { name: '設定' }),
  ).toBeVisible()
  await ctx.close()
})

test('店家資訊:店主在設定頁填寫 → 顧客預約頁顯示地址(地圖連結)與電話(撥號連結)', async ({
  page,
  browser,
  request,
}) => {
  const shop = await createShop(request)
  await signIn(page, shop.owner.email)
  await page.goto(`/admin/${shop.slug}/settings`)
  const form = page.locator('form', { has: page.getByLabel('店家簡介') })
  await form.getByLabel('店家簡介').fill('專做自然捲。\n歡迎預約!')
  await form.getByLabel('地址').fill('台北市中山區南京東路一段 1 號')
  // 電話格式錯誤:前端擋下
  await form.getByLabel('電話').fill('call me')
  await expect(form.getByText(/只能包含數字/)).toBeVisible()
  await expect(form.getByRole('button', { name: '儲存' })).toBeDisabled()
  await form.getByLabel('電話').fill('+886 2-1234-5678')
  await form.getByRole('button', { name: '儲存' }).click()
  await expect(form.getByText('已儲存。')).toBeVisible()

  const guest = await browser.newContext()
  const guestPage = await guest.newPage()
  await guestPage.goto(`/s/${shop.slug}`)
  await expect(guestPage.getByText('專做自然捲。')).toBeVisible()
  await expect(guestPage.getByRole('link', { name: /南京東路/ })).toHaveAttribute(
    'href',
    /google\.com\/maps\/search\/\?api=1&query=/,
  )
  await expect(guestPage.getByRole('link', { name: /2-1234-5678/ })).toHaveAttribute(
    'href',
    'tel:+886212345678',
  )
  await guest.close()
})

test.describe('行動版(手機寬度):沒有橫向捲動、主要操作可見', () => {
  test.use({ viewport: { width: 375, height: 667 } })

  const noHorizontalScroll = async (page: import('@playwright/test').Page) => {
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    )
    expect(overflow).toBeLessThanOrEqual(0)
  }

  test('顧客預約頁與選時間', async ({ page, request }) => {
    const shop = await createShop(request)
    await page.goto(`/s/${shop.slug}`)
    await expect(page.getByRole('link', { name: new RegExp(shop.serviceName) })).toBeVisible()
    await noHorizontalScroll(page)
    await page.getByRole('link', { name: new RegExp(shop.serviceName) }).click()
    await expect(page.locator('button.slot').first()).toBeVisible()
    await noHorizontalScroll(page)
  })

  test('店家後台:列表、預約、團隊、設定', async ({ page, request }) => {
    const shop = await createShop(request)
    await signIn(page, shop.owner.email)
    for (const path of ['', '/bookings', '/services', '/team', '/settings']) {
      await page.goto(`/admin/${shop.slug}${path}`)
      await expect(page.locator('main, .admin-content').first()).toBeVisible()
      await noHorizontalScroll(page)
    }
    await page.goto('/admin')
    await expect(page.getByRole('heading', { name: '我的店家' })).toBeVisible()
    await noHorizontalScroll(page)
  })
})

test.describe('鍵盤操作', () => {
  test('登入只用鍵盤:Tab 依序走過欄位、Enter 送出', async ({ page, request }) => {
    const shop = await createShop(request)
    await page.goto('/admin/login')
    await page.getByLabel('Email').focus()
    await page.keyboard.type(shop.owner.email)
    await page.keyboard.press('Tab')
    await expect(page.getByLabel('密碼')).toBeFocused()
    await page.keyboard.type('password-e2e-1')
    await page.keyboard.press('Enter')
    await expect(page.getByRole('heading', { name: '我的店家' })).toBeVisible()
  })

  test('對話框:開啟後焦點在裡面、Tab 不會跑到背景、Esc 關閉並把焦點還給觸發的按鈕', async ({
    page,
    request,
  }) => {
    const shop = await createShop(request)
    await signIn(page, shop.owner.email)
    await page.goto('/admin')
    const trigger = page.getByRole('button', { name: '登出所有裝置' })
    await trigger.focus()
    await page.keyboard.press('Enter')
    const dialog = page.getByRole('dialog')
    await expect(dialog).toBeVisible()
    // 焦點在對話框裡,連按 Tab 也不會離開
    // (原生 modal dialog 把背景設為 inert:焦點只會在對話框內循環,或暫時交給瀏覽器介面 → activeElement 是 body)
    for (let i = 0; i < 6; i++) {
      await page.keyboard.press('Tab')
      const inside = await dialog.evaluate(
        (d) => d.contains(document.activeElement) || document.activeElement === document.body,
      )
      expect(inside).toBe(true)
    }
    await page.keyboard.press('Escape')
    await expect(dialog).toBeHidden()
    await expect(trigger).toBeFocused()
  })
})
