import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

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
    restoreMocks: true,
    // fetch 在 Node 不接受相對網址,測試時給一個絕對的基底(MSW 會攔截)
    env: { VITE_API_BASE: 'http://api.test' },
  },
})
