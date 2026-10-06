import { QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { HttpResponse, http } from 'msw'
import { beforeEach, describe, expect, it } from 'vitest'
import { createQueryClient } from '../../queryClient'
import { freezeNow } from '../../test/freeze'
import { API, error, server, taipei } from '../../test/server'
import type { TimeOff, TimeOffPage } from '../types'
import { TimeOffSection } from './TimeOffSection'

beforeEach(() => freezeNow())

const LIST_URL = `${API}/t/demo-salon/members/u1/time-off`

const off = (i: number, over: Partial<TimeOff> = {}): TimeOff => ({
  id: `t${i}`,
  user_id: 'u1',
  starts_at: taipei('2026-11-01', '09:00'),
  ends_at: taipei('2026-11-01', '18:00'),
  reason: `原因${i}`,
  ...over,
})

/** 依 offset / past 切出一頁,並記下每次請求的查詢參數 */
function serve(all: { upcoming: TimeOff[]; past?: TimeOff[] }, pageSize = 2) {
  const seen: { past: string | null; offset: string | null }[] = []
  server.use(
    http.get(LIST_URL, ({ request }) => {
      const q = new URL(request.url).searchParams
      seen.push({ past: q.get('past'), offset: q.get('offset') })
      const list = q.get('past') === 'true' ? (all.past ?? []) : all.upcoming
      const offset = Number(q.get('offset') ?? 0)
      const items = list.slice(offset, offset + pageSize)
      const page: TimeOffPage = {
        items,
        limit: pageSize,
        offset,
        has_more: offset + pageSize < list.length,
      }
      return HttpResponse.json(page)
    }),
  )
  return seen
}

function renderSection(editable = true) {
  const client = createQueryClient({ testing: true })
  return render(
    <QueryClientProvider client={client}>
      <TimeOffSection slug="demo-salon" userId="u1" timezone="Asia/Taipei" editable={editable} />
    </QueryClientProvider>,
  )
}

describe('休假清單', () => {
  it('分頁:先載入第一頁,「載入更多」依 offset 接著抓,抓完後按鈕消失', async () => {
    const seen = serve({ upcoming: [off(1), off(2), off(3), off(4), off(5)] })
    renderSection()
    const user = userEvent.setup()

    expect(await screen.findByText('原因1')).toBeInTheDocument()
    expect(screen.getByText('原因2')).toBeInTheDocument()
    expect(screen.queryByText('原因3')).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: '載入更多' }))
    expect(await screen.findByText('原因4')).toBeInTheDocument()
    expect(screen.getByText('原因1')).toBeInTheDocument() // 前面的還在,是累加不是取代

    await user.click(screen.getByRole('button', { name: '載入更多' }))
    expect(await screen.findByText('原因5')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '載入更多' })).not.toBeInTheDocument()
    expect(seen.map((s) => s.offset)).toEqual([null, '2', '4'].map((o) => (o === null ? '0' : o)))
    expect(seen.every((s) => s.past === null)).toBe(true) // 預設不要已結束的
  })

  it('剛好一頁、後面沒有 → 沒有「載入更多」', async () => {
    serve({ upcoming: [off(1), off(2)] })
    renderSection()
    expect(await screen.findByText('原因2')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '載入更多' })).not.toBeInTheDocument()
  })

  it('勾「只看已結束的」→ 改看已結束的(標示「已結束」),取消勾選回到未結束的', async () => {
    const seen = serve({
      upcoming: [off(1)],
      past: [
        off(8, {
          starts_at: taipei('2026-09-01', '09:00'),
          ends_at: taipei('2026-09-02', '09:00'),
          reason: '舊的',
        }),
      ],
    })
    renderSection()
    const user = userEvent.setup()
    expect(await screen.findByText('原因1')).toBeInTheDocument()

    await user.click(screen.getByRole('checkbox', { name: '只看已結束的' }))
    expect(await screen.findByText('舊的')).toBeInTheDocument()
    expect(screen.queryByText('原因1')).not.toBeInTheDocument()
    expect(screen.getByText('已結束')).toBeInTheDocument()
    expect(seen.at(-1)).toEqual({ past: 'true', offset: '0' })

    await user.click(screen.getByRole('checkbox', { name: '只看已結束的' }))
    expect(await screen.findByText('原因1')).toBeInTheDocument()
    expect(screen.queryByText('舊的')).not.toBeInTheDocument()
  })

  it('空狀態:兩種清單的說明不同', async () => {
    serve({ upcoming: [], past: [] })
    renderSection()
    const user = userEvent.setup()
    expect(await screen.findByText('沒有設定休假。')).toBeInTheDocument()
    await user.click(screen.getByRole('checkbox', { name: '只看已結束的' }))
    expect(await screen.findByText('沒有已結束的休假。')).toBeInTheDocument()
  })

  it('載入更多失敗 → 顯示錯誤,已載入的還在,可以再按一次', async () => {
    serve({ upcoming: [off(1), off(2), off(3)] })
    let fail = true
    server.use(
      http.get(LIST_URL, ({ request }) => {
        const offset = Number(new URL(request.url).searchParams.get('offset') ?? 0)
        if (offset > 0 && fail) return error(500, 'x')
        const items = offset === 0 ? [off(1), off(2)] : [off(3)]
        return HttpResponse.json({ items, limit: 2, offset, has_more: offset === 0 })
      }),
    )
    renderSection()
    const user = userEvent.setup()
    expect(await screen.findByText('原因2')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '載入更多' }))
    expect(await screen.findByText('載入更多失敗,請再試一次。')).toBeInTheDocument()
    expect(screen.getByText('原因1')).toBeInTheDocument()

    fail = false
    await user.click(screen.getByRole('button', { name: '載入更多' }))
    expect(await screen.findByText('原因3')).toBeInTheDocument()
  })

  it('刪除一筆後重新載入清單', async () => {
    let items = [off(1), off(2)]
    server.use(
      http.get(LIST_URL, () => HttpResponse.json({ items, limit: 50, offset: 0, has_more: false })),
      http.delete(`${API}/t/demo-salon/time-off/t1`, () => {
        items = items.filter((i) => i.id !== 't1')
        return new HttpResponse(null, { status: 204 })
      }),
    )
    renderSection()
    const user = userEvent.setup()
    const row = (await screen.findByText('原因1')).closest('li')!
    await user.click(within(row).getByRole('button', { name: '刪除' }))
    await user.click(
      within(await screen.findByRole('dialog')).getByRole('button', { name: '刪除' }),
    )
    await waitFor(() => expect(screen.queryByText('原因1')).not.toBeInTheDocument())
    expect(screen.getByText('原因2')).toBeInTheDocument()
  })

  it('不能編輯的人(看別人的休假)沒有新增 / 刪除', async () => {
    serve({ upcoming: [off(1)] })
    renderSection(false)
    expect(await screen.findByText('原因1')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '新增休假' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '刪除' })).not.toBeInTheDocument()
  })
})
