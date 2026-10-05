import react from '@vitejs/plugin-react'
import { configDefaults, defineConfig } from 'vitest/config'

// 開發時前端與後端不同埠:把 /api 代理到後端並去掉前綴,瀏覽器看到的是同源,不需要 CORS。
// 正式環境由反向代理做同樣的事(見 docs/deployment.md)。
const backend = process.env.VITE_BACKEND ?? 'http://127.0.0.1:3001'

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      '/api': { target: backend, rewrite: (path) => path.replace(/^\/api/, '') },
    },
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    css: false,
    testTimeout: 10_000,
    // e2e/ 是 Playwright 的測試(用 `npm run e2e`),不能讓 Vitest 去執行
    exclude: [...configDefaults.exclude, 'e2e/**'],
    restoreMocks: true,
    // fetch 在 Node 不接受相對網址,測試時給一個絕對的基底(MSW 會攔截)
    env: { VITE_API_BASE: 'http://api.test' },
  },
})
