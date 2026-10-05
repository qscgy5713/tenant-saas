import { expect, test } from '@playwright/test'
import {
  bookAsOwner,
  createShop,
  signIn,
  taipeiAt,
  taipeiDay,
  uniq,
  waitForMail,
  weekdayOf,
} from './helpers'

/** 週一開始的欄位索引 */
const columnOf = (ymd: string) => (weekdayOf(ymd) + 6) % 7

test('店家在週日曆把預約拖到別天的別個時間 → 確認 → 改期,顧客收到通知信', async ({
  page,
  request,
}) => {
  const shop = await createShop(request)
  const customerEmail = `cust-${uniq()}@example.com`
  const from = taipeiDay(2)
  // 目標是同一週的相鄰一天,而且一定在未來(週日往前一天,其他往後一天)
  const to = weekdayOf(from) === 0 ? taipeiDay(1) : taipeiDay(3)
  await bookAsOwner(request, shop, taipeiAt(from, '14:00'), {
    name: '王小明',
    email: customerEmail,
  })

  await signIn(page, shop.owner.email)
  await page.goto(`/admin/${shop.slug}/bookings?view=week&date=${from}`)
  const columns = page.locator('.cal-col')
  const block = columns.nth(columnOf(from)).locator('.cal-item', { hasText: '王小明' })
  await expect(block).toBeVisible()
  await expect(block).toHaveAttribute('draggable', 'true')

  // 週日曆預設 09:00 起、每小時 48px;抓在區塊正中間(往下 24px),
  // 所以放開點 = 新開始時間的位置 + 24px。16:00 → (16-9)*48 + 24 = 360
  await block.dragTo(columns.nth(columnOf(to)), { targetPosition: { x: 20, y: 360 } })

  const dialog = page.getByRole('dialog')
  await expect(dialog.getByText(/14:00 改到.*16:00/)).toBeVisible()
  await dialog.getByRole('button', { name: '確定改期' }).click()
  await expect(page.getByText('已改期,並寄出通知信給顧客。')).toBeVisible()

  // 預約真的移到新的一天與新的時間
  await expect(
    columns.nth(columnOf(to)).locator('.cal-item', { hasText: '王小明' }).getByText('16:00'),
  ).toBeVisible()
  await expect(columns.nth(columnOf(from)).locator('.cal-item')).toHaveCount(0)

  const mail = await waitForMail(request, customerEmail, '預約時間已變更')
  expect(mail.text).toContain('16:00')
})

test('拖到營業時間外 → 後端拒絕,原因顯示在確認視窗,預約沒被改動', async ({ page, request }) => {
  const shop = await createShop(request)
  const from = taipeiDay(2)
  const to = weekdayOf(from) === 0 ? taipeiDay(1) : taipeiDay(3)
  await bookAsOwner(request, shop, taipeiAt(from, '14:00'), {
    name: '王小明',
    email: `cust-${uniq()}@example.com`,
  })

  await signIn(page, shop.owner.email)
  await page.goto(`/admin/${shop.slug}/bookings?view=week&date=${from}`)
  const columns = page.locator('.cal-col')
  const block = columns.nth(columnOf(from)).locator('.cal-item', { hasText: '王小明' })
  // 17:30 開始、一小時 → 超過 18:00 打烊。(17.5-9)*48 + 24 = 432
  await block.dragTo(columns.nth(columnOf(to)), { targetPosition: { x: 20, y: 432 } })

  const dialog = page.getByRole('dialog')
  await dialog.getByRole('button', { name: '確定改期' }).click()
  await expect(dialog.getByRole('alert')).toContainText('無法預約')
  await expect(dialog).toBeVisible()

  await page.keyboard.press('Escape')
  await expect(
    columns.nth(columnOf(from)).locator('.cal-item', { hasText: '王小明' }),
  ).toBeVisible()
})
