import { expect, test } from '@playwright/test'
import { PASSWORD, createShop, linkIn, loginViaUi, signIn, waitForMail } from './helpers'

test('連續輸錯被鎖定 → 通知信 → 忘記密碼重設 → 解鎖並能用新密碼登入', async ({ page, request }) => {
  const { owner } = await createShop(request)
  const newPassword = 'brand-new-password-2'

  // 連錯 5 次
  for (let i = 0; i < 5; i++) {
    await loginViaUi(page, owner.email, `wrong-password-${i}`)
    await expect(page.getByRole('alert').first()).toContainText('Email 或密碼錯誤')
  }
  // 鎖定中:連正確密碼都不收,而且看起來跟輸錯密碼一模一樣(不洩漏帳號被鎖)
  await loginViaUi(page, owner.email, PASSWORD)
  await expect(page.getByRole('alert').first()).toContainText('Email 或密碼錯誤')
  await expect(page.getByRole('heading', { name: '登入' })).toBeVisible()
  const lockMail = await waitForMail(request, owner.email, '暫時鎖定')
  expect(lockMail.text).toContain('15 分鐘')

  // 忘記密碼不受鎖定影響
  await page.goto('/admin/forgot')
  await page.getByLabel('Email').fill(owner.email)
  await page.getByRole('button', { name: '寄送重設信' }).click()
  await expect(page.getByRole('heading', { name: '請查看信箱' })).toBeVisible()
  const resetMail = await waitForMail(request, owner.email, '重設您的密碼')

  await page.goto(linkIn(resetMail, '/admin/reset'))
  await page.getByLabel('新密碼').fill(newPassword)
  await page.getByLabel('再輸入一次').fill(newPassword)
  await page.getByRole('button', { name: '更新密碼' }).click()
  await expect(page.getByRole('heading', { name: '密碼已更新' })).toBeVisible()

  // 舊密碼不能用,新密碼可以(重設同時解鎖)
  await loginViaUi(page, owner.email, PASSWORD)
  await expect(page.getByRole('alert').first()).toContainText('Email 或密碼錯誤')
  await signIn(page, owner.email, newPassword)
  await expect(page.getByRole('heading', { name: '我的店家' })).toBeVisible()

  // 重設連結只能用一次
  await page.goto(linkIn(resetMail, '/admin/reset'))
  await page.getByLabel('新密碼').fill('another-password-3')
  await page.getByLabel('再輸入一次').fill('another-password-3')
  await page.getByRole('button', { name: '更新密碼' }).click()
  await expect(page.getByText(/無效|過期/).first()).toBeVisible()
})
