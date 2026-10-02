import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { HttpResponse, http } from 'msw'
import { beforeEach, describe, expect, it } from 'vitest'
import { freezeNow } from '../test/freeze'
import { renderApp } from '../test/render'
import { API, booking, error, server, standardSlots, taipei } from '../test/server'

beforeEach(() => freezeNow())

const TOKEN = 'a'.repeat(64)
const PATH = `/bookings/${TOKEN}`
const url = (suffix = '') => `${API}/public/bookings/${TOKEN}${suffix}`

describe('預約管理頁(信中的連結)', () => {
  it('待確認:顯示尚未成立,而且不會自動確認(信件掃描器會先點開連結)', async () => {
    let confirmed = 0
    server.use(
      http.get(url(), () => HttpResponse.json(booking({ status: 'pending' }))),
      http.post(url('/confirm'), () => ((confirmed += 1), HttpResponse.json(booking()))),
    )
    renderApp(PATH)
    expect(await screen.findByText('待確認')).toBeInTheDocument()
    expect(screen.getByText(/還沒成立/)).toBeInTheDocument()
    await new Promise((r) => setTimeout(r, 100))
    expect(confirmed).toBe(0)
  })

  it('按下「確認預約」→ 狀態更新、顯示成功訊息、出現改期 / 取消', async () => {
    let state = booking({ status: 'pending' })
    server.use(
      http.get(url(), () => HttpResponse.json(state)),
      http.post(url('/confirm'), () => ((state = booking()), HttpResponse.json(state))),
    )
    renderApp(PATH)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '確認預約' }))
    expect(await screen.findByText(/預約已確認/)).toBeInTheDocument()
    expect(screen.getByText('已確認')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '改期' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '確認預約' })).not.toBeInTheDocument()
  })

  it('確認時被搶走(409)→ 顯示原因,預約仍是待確認', async () => {
    server.use(
      http.get(url(), () => HttpResponse.json(booking({ status: 'pending' }))),
      http.post(url('/confirm'), () =>
        error(409, '這個時段在您確認之前已被其他人預約,請重新選擇時間'),
      ),
    )
    renderApp(PATH)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '確認預約' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('已被其他人預約')
  })

  it('取消:先出現確認面板,按「先不要」不會送出;「確定取消」才送出', async () => {
    let state = booking()
    let cancels = 0
    server.use(
      http.get(url(), () => HttpResponse.json(state)),
      http.post(
        url('/cancel'),
        () => (
          (cancels += 1),
          (state = booking({ status: 'cancelled' })),
          HttpResponse.json(state)
        ),
      ),
    )
    renderApp(PATH)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '取消預約' }))
    await user.click(screen.getByRole('button', { name: '先不要' }))
    expect(cancels).toBe(0)

    await user.click(screen.getByRole('button', { name: '取消預約' }))
    await user.click(screen.getByRole('button', { name: '確定取消' }))
    expect(await screen.findByText('預約已取消。')).toBeInTheDocument()
    expect(screen.getByText('已取消')).toBeInTheDocument()
    expect(cancels).toBe(1)
    expect(screen.queryByRole('button', { name: '改期' })).not.toBeInTheDocument()
  })

  it('待確認的預約也能取消,按鈕文字是「取消這筆申請」', async () => {
    server.use(http.get(url(), () => HttpResponse.json(booking({ status: 'pending' }))))
    renderApp(PATH)
    expect(await screen.findByRole('button', { name: '取消這筆申請' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '改期' })).not.toBeInTheDocument() // 待確認不能改期
  })

  it('改期:時段來自「依預約」的端點(排除自己),送出新的開始時間', async () => {
    const queries: URLSearchParams[] = []
    let shopWide = 0
    let rescheduled: unknown
    server.use(
      http.get(url(), () => HttpResponse.json(booking())),
      // 這個端點由後端排除預約自己,所以包含自己原本的 10:00 / 10:15
      http.get(url('/availability'), ({ request }) => {
        const q = new URL(request.url).searchParams
        queries.push(q)
        return HttpResponse.json({
          timezone: 'Asia/Taipei',
          slots: standardSlots(q.get('from')!, q.get('to')!),
        })
      }),
      http.get(
        `${API}/public/shops/demo-salon/availability`,
        () => ((shopWide += 1), HttpResponse.json({ timezone: 'Asia/Taipei', slots: [] })),
      ),
      http.post(url('/reschedule'), async ({ request }) => {
        rescheduled = await request.json()
        return HttpResponse.json(
          booking({
            starts_at: taipei('2026-10-06', '10:30'),
            ends_at: taipei('2026-10-06', '11:30'),
          }),
        )
      }),
    )
    renderApp(PATH)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: '改期' }))
    await user.click(await screen.findByRole('button', { name: '10:30' }))

    // 不帶服務 / 員工參數(固定是這筆預約的),也沒有打店家共用的端點(那個會把自己當成忙碌)
    expect(queries[0].get('from')).toBeTruthy()
    expect(queries[0].has('service_id') || queries[0].has('staff_id')).toBe(false)
    expect(shopWide).toBe(0)
    expect(screen.queryByRole('radio')).not.toBeInTheDocument() // 改期不能換人

    await user.click(screen.getByRole('button', { name: /改到.*10:30/ }))
    expect(await screen.findByText('已改期。')).toBeInTheDocument()
    expect(rescheduled).toEqual({ start: taipei('2026-10-05', '10:30') })
  })

  it('已確認且還沒開始 → 提供「加到行事曆」下載連結;待確認的沒有', async () => {
    server.use(http.get(url(), () => HttpResponse.json(booking())))
    const first = renderApp(PATH)
    const link = await screen.findByRole('link', { name: '加到行事曆' })
    expect(link).toHaveAttribute(
      'href',
      expect.stringMatching(/\/public\/bookings\/.+\/calendar\.ics$/),
    )
    expect(link).toHaveAttribute('download', 'booking.ics')
    first.unmount()

    server.use(http.get(url(), () => HttpResponse.json(booking({ status: 'pending' }))))
    renderApp(PATH)
    await screen.findByRole('button', { name: '確認預約' })
    expect(screen.queryByRole('link', { name: '加到行事曆' })).not.toBeInTheDocument()
  })

  it('預約時間已過 → 不提供改期 / 取消', async () => {
    server.use(
      http.get(url(), () =>
        HttpResponse.json(
          booking({
            starts_at: taipei('2026-10-05', '08:00'),
            ends_at: taipei('2026-10-05', '09:00'),
          }),
        ),
      ),
    )
    renderApp(PATH)
    expect(await screen.findByText(/預約時間已過/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '改期' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '取消預約' })).not.toBeInTheDocument()
  })

  it.each([
    ['cancelled', '這筆預約已取消。'],
    ['completed', '這筆預約已經結束。'],
    ['no_show', '這筆預約已經結束。'],
  ] as const)('狀態 %s → 說明並且沒有任何操作', async (status, text) => {
    server.use(http.get(url(), () => HttpResponse.json(booking({ status }))))
    renderApp(PATH)
    expect(await screen.findByText(text)).toBeInTheDocument()
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })

  it('連結無效(404)→ 說明可能原因,不提供重試', async () => {
    server.use(http.get(url(), () => error(404, '找不到資源')))
    renderApp(PATH)
    expect(await screen.findByText('找不到這筆預約')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '重試' })).not.toBeInTheDocument()
  })

  it('顯示的是店家時區的時間,不是瀏覽器的', async () => {
    server.use(
      http.get(url(), () =>
        HttpResponse.json(
          booking({
            timezone: 'America/New_York',
            starts_at: '2026-10-08T18:00:00Z',
            ends_at: '2026-10-08T19:00:00Z',
          }),
        ),
      ),
    )
    renderApp(PATH)
    expect(await screen.findByText(/14:00 – 15:00/)).toBeInTheDocument() // 紐約 EDT = UTC-4
    expect(screen.getByText(/GMT-4/)).toBeInTheDocument()
    await waitFor(() => expect(document.title).toContain('森林系髮廊'))
  })
})
