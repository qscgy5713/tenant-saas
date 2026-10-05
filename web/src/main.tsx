import { QueryClientProvider } from '@tanstack/react-query'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserRouter } from 'react-router-dom'
import { App } from './App'
import { restoreSession } from './admin/session'
import { createQueryClient } from './queryClient'
import './styles.css'

const queryClient = createQueryClient()
// 登入靠 HttpOnly cookie,前端看不到:向後端問「我現在登入了嗎」
void restoreSession()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <App />
      </BrowserRouter>
    </QueryClientProvider>
  </StrictMode>,
)
