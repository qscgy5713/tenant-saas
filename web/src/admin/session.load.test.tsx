// 這個檔案的測試會 vi.resetModules() 模擬「全新載入的頁面」,
// 會讓之後延遲載入的元件拿到另一份 session 模組,所以必須獨立成一個檔案(vitest 每個檔案各自隔離)。
import { render, screen } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { describe, expect, it, vi } from 'vitest'
import { USER } from '../test/admin'
import { API, error, server } from '../test/server'

describe('載入頁面時的登入狀態', () => {
  it('升級時丟掉舊版存在 localStorage 的 JWT', async () => {
    localStorage.setItem(
      'tenant-saas.session',
      JSON.stringify({ token: 'old.jwt.value', user: USER }),
    )
    vi.resetModules()
    await import('./session') // 模組載入時清除
    expect(localStorage.getItem('tenant-saas.session')).toBeNull()
  })
})

describe('重新整理:登入狀態確認前不能閃一下登入頁', () => {
  async function fresh() {
    vi.resetModules() // 全新載入的頁面:狀態是 loading
    const [{ RequireAuth }, session] = await Promise.all([import('./auth'), import('./session')])
    render(
      <MemoryRouter initialEntries={['/admin/x']}>
        <Routes>
          <Route path="/admin/login" element={<p>登入頁</p>} />
          <Route element={<RequireAuth />}>
            <Route path="/admin/x" element={<p>後台內容</p>} />
          </Route>
        </Routes>
      </MemoryRouter>,
    )
    return session
  }

  it('確認中顯示載入;cookie 有效 → 進後台', async () => {
    server.use(http.get(`${API}/auth/me`, () => HttpResponse.json(USER)))
    const session = await fresh()
    expect(session.getStatus()).toBe('loading')
    expect(screen.getByRole('status')).toHaveTextContent('載入中')
    expect(screen.queryByText('登入頁')).not.toBeInTheDocument()
    expect(screen.queryByText('後台內容')).not.toBeInTheDocument()

    await session.restoreSession()
    expect(await screen.findByText('後台內容')).toBeInTheDocument()
  })

  it('確認中顯示載入;沒有登入 → 導向登入頁', async () => {
    server.use(http.get(`${API}/auth/me`, () => error(401, '未登入或憑證無效')))
    const session = await fresh()
    expect(screen.queryByText('登入頁')).not.toBeInTheDocument()
    await session.restoreSession()
    expect(await screen.findByText('登入頁')).toBeInTheDocument()
  })
})
