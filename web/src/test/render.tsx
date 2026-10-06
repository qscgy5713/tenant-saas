import { QueryClientProvider } from '@tanstack/react-query'
import { render } from '@testing-library/react'
import { StrictMode } from 'react'
import { MemoryRouter } from 'react-router-dom'
import { App } from '../App'
import { createQueryClient } from '../queryClient'

export function renderApp(path: string, { strict = false }: { strict?: boolean } = {}) {
  // 與正式環境同一份設定(含全域 401 處理),只是關掉重試
  const client = createQueryClient({ testing: true })
  const tree = (
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[path]}>
        <App />
      </MemoryRouter>
    </QueryClientProvider>
  )
  // 開發模式的 StrictMode 會讓 effect 跑兩次:一次性的動作(驗證連結)要能承受
  const utils = render(strict ? <StrictMode>{tree}</StrictMode> : tree)
  return { ...utils, client }
}
