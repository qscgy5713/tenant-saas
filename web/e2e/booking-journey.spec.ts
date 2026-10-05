import { expect, test } from '@playwright/test'
import { countMails, createShop, linkIn, uniq, waitForMail } from './helpers'

test('顧客預約 → 收信確認 → 管理頁取消 → 負責的員工收到通知(顧客不重複收信)', async ({
  page,
  request,
}) => {
  const shop = await createShop(request)
  const customerEmail = `cust-${uniq()}@example.com`

  // 1. 選服務、選時間
  await page.goto(`/s/${shop.slug}`)
  await page.getByRole('link', { name: new RegExp(shop.serviceName) }).click()
  await expect(page.getByRole('heading', { name: '選擇日期與時間' })).toBeVisible()
  const slot = page.locator('button.slot').first()
  await expect(slot).toBeVisible()
  await slot.click()
  await page.getByRole('button', { name: /下一步:填寫資料/ }).click()

  // 2. 填資料、送出 —— 預約此時尚未成立
  await page.getByLabel('姓名').fill('王小明')
  await page.getByLabel('Email').fill(customerEmail)
  await page.getByRole('button', { name: '送出預約申請' }).click()
  await expect(page.getByRole('heading', { name: '請到信箱確認預約' })).toBeVisible()
  await expect(page.getByText('預約尚未成立')).toBeVisible()

  // 3. 信真的寄到了(走 SMTP),按信中的連結確認
  const confirmMail = await waitForMail(request, customerEmail, '請確認')
  await page.goto(linkIn(confirmMail, '/bookings/'))
  await expect(page.getByText('這筆預約還沒成立')).toBeVisible()
  await page.getByRole('button', { name: '確認預約' }).click()
  await expect(page.getByText('這筆預約還沒成立')).toBeHidden()
  await expect(page.getByRole('button', { name: '取消預約' })).toBeVisible()
  await waitForMail(request, customerEmail, '已確認')

  // 4. 顧客自己取消
  await page.getByRole('button', { name: '取消預約' }).click()
  await page.getByRole('button', { name: '確定取消' }).click()
  await expect(page.getByText('這筆預約已取消')).toBeVisible()

  // 5. 負責的員工(這裡就是店主)收到取消通知;顧客不會再收到一封「取消」
  const notice = await waitForMail(request, shop.owner.email, '預約已取消')
  expect(notice.text).toContain('顧客自己取消')
  expect(notice.text).toContain('王小明')
  expect(await countMails(request, customerEmail, '預約已取消')).toBe(0)
})
