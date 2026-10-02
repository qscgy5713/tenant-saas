import { HttpResponse, http } from 'msw'
import { describe, expect, it } from 'vitest'
import { server, API } from '../test/server'
import { ApiError, api } from './client'

const failure = async (path: string) => {
  try {
    await api(path)
  } catch (e) {
    return e as ApiError
  }
  throw new Error('應該要失敗')
}

describe('api()', () => {
  it('成功時回傳 JSON', async () => {
    server.use(http.get(`${API}/x`, () => HttpResponse.json({ ok: 1 })))
    expect(await api('/x')).toEqual({ ok: 1 })
  })

  it('4xx:直接用後端的中文訊息,並帶上請求 ID', async () => {
    server.use(
      http.get(`${API}/x`, () =>
        HttpResponse.json(
          { error: '所選時段無法預約' },
          { status: 409, headers: { 'x-request-id': 'req-9' } },
        ),
      ),
    )
    const e = await failure('/x')
    expect(e).toBeInstanceOf(ApiError)
    expect([e.status, e.message, e.requestId]).toEqual([409, '所選時段無法預約', 'req-9'])
    expect(e.isClientError).toBe(true)
  })

  it('5xx:不把後端的內容顯示給使用者(可能是內部細節),但保留請求 ID 供查詢', async () => {
    server.use(
      http.get(`${API}/x`, () =>
        HttpResponse.json(
          { error: 'connection to 10.0.0.5 refused' },
          { status: 500, headers: { 'x-request-id': 'req-5' } },
        ),
      ),
    )
    const e = await failure('/x')
    expect(e.message).not.toContain('10.0.0.5')
    expect(e.requestId).toBe('req-5')
    expect(e.isClientError).toBe(false)
  })

  it('網路中斷 → 狀態 0 的友善訊息,不是原始的 TypeError', async () => {
    server.use(http.get(`${API}/x`, () => HttpResponse.error()))
    const e = await failure('/x')
    expect(e.status).toBe(0)
    expect(e.message).toContain('無法連線')
    expect(e.isClientError).toBe(false)
  })

  it('錯誤內容不是 JSON(例如代理伺服器的 HTML 錯誤頁)→ 預設訊息', async () => {
    server.use(
      http.get(`${API}/x`, () => new HttpResponse('<html>Bad Gateway</html>', { status: 404 })),
    )
    const e = await failure('/x')
    expect(e.status).toBe(404)
    expect(e.message).not.toContain('<html>')
  })

  it('錯誤 JSON 沒有 error 欄位 → 預設訊息', async () => {
    server.use(http.get(`${API}/x`, () => HttpResponse.json({ detail: 1 }, { status: 400 })))
    expect((await failure('/x')).message).toBeTruthy()
  })
})
