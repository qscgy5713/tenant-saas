import { QueryClientProvider } from '@tanstack/react-query'
import { render } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { App } from '../App'
import { createQueryClient } from '../queryClient'

export function renderApp(path: string) {
  // 與正式環境同一份設定(含全域 401 處理),只是關掉重試
  const client = createQueryClient({ testing: true })
  const utils = render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[path]}>
        <App />
      </MemoryRouter>
    </QueryClientProvider>,
  )
  return { ...utils, client }
}
