import { expect, test } from '@playwright/test'
import {
  addStaff,
  bookAsOwner,
  createShop,
  signIn,
  makeBookable,
  taipeiAt,
  taipeiDay,
  uniq,
  waitForMail,
} from './helpers'

test('停用員工:對方進不了這家店,重新啟用後回來', async ({ page, browser, request }) => {
  const shop = await createShop(request)
  const staff = await addStaff(request, shop, '員工小安')
  await makeBookable(request, shop, staff, shop.serviceId)
  const bookPage = `/s/${shop.slug}/book/${shop.serviceId}`
  // 顧客的預約頁:有兩位可選時才會出現「選人員」
  const picker = (p: typeof page) => p.getByRole('radio', { name: '員工小安' })

  await page.goto(bookPage)
  await expect(picker(page)).toBeVisible()

  await signIn(page, shop.owner.email)
  await page.goto(`/admin/${shop.slug}/team`)
  const row = page.locator('li.member-row', { hasText: staff.email })
  await expect(row).toBeVisible()

  await row.getByRole('button', { name: '停用' }).click()
  await page.getByRole('dialog').getByRole('button', { name: '停用' }).click()
  await expect(row.getByText('已停用')).toBeVisible()
  await expect(row.getByRole('button', { name: '重新啟用' })).toBeVisible()

  // 員工那邊:登入得了,但這家店不在他的清單裡
  const staffContext = await browser.newContext()
  const staffPage = await staffContext.newPage()
  await signIn(staffPage, staff.email)
  await expect(staffPage.getByRole('heading', { name: '我的店家' })).toBeVisible()
  await expect(staffPage.getByText(shop.name)).toHaveCount(0)
  // 直接打網址也進不去。注意要先等到「找不到」這個正向訊號 ——
  // 只斷言「看不到店名」,頁面還在載入時就會立刻成立,什麼都沒驗證到
  await staffPage.goto(`/admin/${shop.slug}`)
  await expect(staffPage.getByRole('alert')).toContainText('找不到這家店')
  await expect(staffPage.getByText(shop.name)).toHaveCount(0)

  // 顧客的預約頁也看不到這位員工了(只剩店主一位,不再出現選人員)
  await page.goto(bookPage)
  await expect(page.getByRole('heading', { name: '選擇日期與時間' })).toBeVisible()
  await expect(picker(page)).toHaveCount(0)

  // 重新啟用 → 員工回來了(後台、預約頁都是)
  await page.goto(`/admin/${shop.slug}/team`)
  await row.getByRole('button', { name: '重新啟用' }).click()
  await expect(row.getByText('已停用')).toBeHidden()
  await page.goto(bookPage)
  await expect(picker(page)).toBeVisible()
  await staffPage.goto('/admin')
  await expect(staffPage.getByText(shop.name)).toBeVisible()
  await staffContext.close()
})

test('還有未來預約的員工不能停用,原因顯示在視窗裡', async ({ page, request }) => {
  const shop = await createShop(request)
  const staff = await addStaff(request, shop)
  await makeBookable(request, shop, staff, shop.serviceId)
  await bookAsOwner(
    request,
    shop,
    taipeiAt(taipeiDay(3), '10:00'),
    { name: '王小明', email: 'someone@example.com' },
    staff.id,
  )

  await signIn(page, shop.owner.email)
  await page.goto(`/admin/${shop.slug}/team`)
  const row = page.locator('li.member-row', { hasText: staff.email })
  await row.getByRole('button', { name: '停用' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByRole('button', { name: '停用' }).click()
  await expect(dialog.getByRole('alert')).toContainText('1 筆未來已確認的預約')
  await expect(row.getByText('已停用')).toHaveCount(0)
})

test('停用時把未來預約改派給店主:停用成功,顧客收到人員變更通知', async ({ page, request }) => {
  const shop = await createShop(request)
  const staff = await addStaff(request, shop)
  await makeBookable(request, shop, staff, shop.serviceId)
  const customerEmail = `cust-${uniq()}@example.com`
  await bookAsOwner(
    request,
    shop,
    taipeiAt(taipeiDay(3), '10:00'),
    { name: '王小明', email: customerEmail },
    staff.id,
  )

  await signIn(page, shop.owner.email)
  await page.goto(`/admin/${shop.slug}/team`)
  const row = page.locator('li.member-row', { hasText: staff.email })
  await row.getByRole('button', { name: '停用' }).click()
  const dialog = page.getByRole('dialog')
  // 這是真實瀏覽器送出的請求:不選改派時 body 是空的、選了才帶 reassign_to
  await dialog.getByRole('combobox', { name: /改派給/ }).selectOption({ label: '店主阿美' })
  await dialog.getByRole('button', { name: '停用' }).click()
  await expect(row.getByText('已停用')).toBeVisible()

  const mail = await waitForMail(request, customerEmail, '人員已變更')
  expect(mail.text).toContain('店主阿美')
  expect(mail.text).toContain('員工小安')
})
