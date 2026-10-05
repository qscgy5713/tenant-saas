import { defineConfig, devices } from '@playwright/test'

// 端對端測試:真的瀏覽器 + 真的後端 + 真的 PostgreSQL + 真的 SMTP(Mailpit),什麼都不 mock。
// 刻意使用與開發環境不同的埠與資料庫,不會碰到你正在開發用的資料。
// 事前需要 PostgreSQL 與 Mailpit(`npm run e2e` 會自動準備,見 e2e/prepare.sh)。
const API_PORT = 3101
const WEB_PORT = 5273
const WEB = `http://localhost:${WEB_PORT}`
const DATABASE_URL = 'postgres://tenant:tenant@localhost:5432/tenant_saas_e2e'

export default defineConfig({
  testDir: './e2e',
  testMatch: '**/*.spec.ts',
  // 每個測試用自己的帳號與店家(隨機代號),互不影響,所以可以平行
  fullyParallel: true,
  workers: process.env.CI ? 2 : undefined,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  timeout: 60_000,
  expect: { timeout: 10_000 },
  use: {
    baseURL: WEB,
    locale: 'zh-TW',
    timezoneId: 'Asia/Taipei',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: [
    {
      // CI 事先建置好,直接執行二進位檔;本機用 cargo run
      command: process.env.E2E_API_CMD ?? 'cargo run --manifest-path ../Cargo.toml',
      url: `http://127.0.0.1:${API_PORT}/health`,
      reuseExistingServer: !process.env.CI,
      timeout: 240_000,
      env: {
        DATABASE_URL,
        BIND_ADDR: `127.0.0.1:${API_PORT}`,
        JWT_SECRET: 'e2e-secret-e2e-secret-e2e-secret-0123456789',
        PUBLIC_BASE_URL: WEB,
        ALLOWED_ORIGINS: WEB,
        SMTP_URL: 'smtp://127.0.0.1:1025',
        MAIL_FROM: 'e2e <e2e@example.com>',
        WORKER_POLL_SECS: '1',
        // 測試會在短時間內建立很多帳號與預約,來源都是同一個 IP
        AUTH_RATE_LIMIT_PER_MIN: '100000',
        PUBLIC_RATE_LIMIT_PER_MIN: '100000',
        MAX_SHOPS_PER_USER: '1000',
        RUST_LOG: 'warn',
      },
    },
    {
      command: `npm run dev -- --port ${WEB_PORT} --strictPort`,
      url: WEB,
      reuseExistingServer: !process.env.CI,
      env: { VITE_BACKEND: `http://127.0.0.1:${API_PORT}` },
    },
  ],
})
