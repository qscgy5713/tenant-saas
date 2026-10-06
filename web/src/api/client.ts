/** API 錯誤。`message` 可直接顯示給使用者;`requestId` 供客服 / 日誌查詢 */
export class ApiError extends Error {
  readonly status: number
  readonly requestId?: string

  constructor(status: number, message: string, requestId?: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.requestId = requestId
  }

  /** 4xx:使用者可以修正或重試的問題(重試同樣的請求不會有不同結果) */
  get isClientError() {
    return this.status >= 400 && this.status < 500
  }
}

const BASE: string = import.meta.env.VITE_API_BASE ?? '/api'

/** 給 <a href> 用的完整 API 路徑(下載檔案等不經 fetch 的情況) */
export const apiUrl = (path: string) => `${BASE}${path}`

const NETWORK_MESSAGE = '無法連線到伺服器,請檢查網路後再試一次。'
const SERVER_MESSAGE = '伺服器發生問題,請稍後再試。'

/** 把失敗的回應轉成 `ApiError`(5xx 只給固定訊息,4xx 用後端的訊息) */
async function failure(res: Response): Promise<ApiError> {
  const requestId = res.headers.get('x-request-id') ?? undefined
  // 5xx 的內容不顯示給使用者(可能是內部細節),只給固定訊息加上請求 ID
  if (res.status >= 500) return new ApiError(res.status, SERVER_MESSAGE, requestId)
  let message = '操作失敗,請稍後再試。'
  try {
    const body: unknown = await res.json()
    if (body && typeof body === 'object' && 'error' in body && typeof body.error === 'string') {
      message = body.error
    }
  } catch {
    // 不是 JSON,沿用預設訊息
  }
  return new ApiError(res.status, message, requestId)
}

/** 後端錯誤格式是 `{ "error": "..." }`,訊息本身就是給使用者看的中文 */
export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  let res: Response
  try {
    res = await fetch(BASE + path, {
      ...init,
      headers: { 'Content-Type': 'application/json', ...init.headers },
    })
  } catch {
    throw new ApiError(0, NETWORK_MESSAGE)
  }

  if (!res.ok) throw await failure(res)

  if (res.status === 204) return undefined as T
  return (await res.json()) as T
}

export function json(body: unknown): RequestInit {
  return { method: 'POST', body: JSON.stringify(body) }
}

/** 下載檔案(匯出)。失敗時與 `api` 一樣丟 `ApiError`,所以畫面能顯示原因,而不是下載到一個錯誤訊息的檔案 */
export async function apiDownload(path: string): Promise<{ blob: Blob; filename: string }> {
  let res: Response
  try {
    res = await fetch(BASE + path)
  } catch {
    throw new ApiError(0, NETWORK_MESSAGE)
  }
  if (!res.ok) throw await failure(res)
  const disposition = res.headers.get('content-disposition') ?? ''
  const filename = /filename="([^"]+)"/.exec(disposition)?.[1] ?? 'download'
  return { blob: await res.blob(), filename }
}
