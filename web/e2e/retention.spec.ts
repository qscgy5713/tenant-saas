import { expect, test } from '@playwright/test'
import { createShop, signIn } from './helpers'

test('刪除店家:輸入代稱確認 → 公開預約頁立即關閉、後台出現橫幅 → 取消後恢復', async ({
  page,
  browser,
  request,
}) => {
  const shop = await createShop(request)
  const publicUrl = `/s/${shop.slug}`

  // 顧客端:營業中
  const guest = await browser.newContext()
  const guestPage = await guest.newPage()
  await guestPage.goto(publicUrl)
  await expect(guestPage.getByRole('heading', { name: shop.name })).toBeVisible()

  await signIn(page, shop.owner.email)
  await page.goto(`/admin/${shop.slug}/settings`)
  await page.getByRole('button', { name: '刪除這家店' }).click()
  const dialog = page.getByRole('dialog')
  // 打錯代稱:前端擋下
  await dialog.getByLabel('網址代稱').fill('not-the-slug')
  await dialog.getByRole('button', { name: '申請刪除' }).click()
  await expect(dialog.getByRole('alert')).toContainText('不符')
  await dialog.getByLabel('網址代稱').fill(shop.slug)
  await dialog.getByRole('button', { name: '申請刪除' }).click()

  await expect(page.getByRole('button', { name: '取消刪除' })).toBeVisible()
  await expect(page.getByText(/這家店已申請刪除/)).toBeVisible()

  // 顧客端:預約頁立刻關閉
  await guestPage.reload()
  await expect(guestPage.getByText('找不到這家店')).toBeVisible()

  // 取消 → 恢復營業
  await page.getByRole('button', { name: '取消刪除' }).click()
  await expect(page.getByRole('button', { name: '刪除這家店' })).toBeVisible()
  await expect(page.getByText(/這家店已申請刪除/)).toHaveCount(0)
  await guestPage.reload()
  await expect(guestPage.getByRole('heading', { name: shop.name })).toBeVisible()
  await guest.close()
})
