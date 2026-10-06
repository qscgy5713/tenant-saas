import {
  expect,
  request as playwrightRequest,
  type APIRequestContext,
  type Page,
} from '@playwright/test'

export const API = 'http://127.0.0.1:3101'
export const MAILPIT = 'http://127.0.0.1:8025'
export const PASSWORD = 'password-e2e-1'

/** 每個測試用自己的代號,帳號與店家互不干擾 */
export const uniq = () => Math.random().toString(36).slice(2, 8)

// ---------- 日期(店家在台北時區) ----------

const TAIPEI_OFFSET_MS = 8 * 3_600_000

/** 今天起算 `offset` 天後,台北的日期 `YYYY-MM-DD` */
export function taipeiDay(offset: number): string {
  return new Date(Date.now() + TAIPEI_OFFSET_MS + offset * 86_400_000).toISOString().slice(0, 10)
}

/** 台北某天某時刻的 UTC ISO 字串 */
export const taipeiAt = (ymd: string, hhmm: string) =>
  new Date(`${ymd}T${hhmm}:00+08:00`).toISOString()

/** 0 = 週日 */
export const weekdayOf = (ymd: string) => new Date(`${ymd}T12:00:00Z`).getUTCDay()

// ---------- 後端 API(只用來準備測試資料;被測的行為一律走畫面) ----------

export interface Account {
  email: string
  password: string
  name: string
  id: string
  token: string
}

async function call<T>(
  request: APIRequestContext,
  method: 'GET' | 'POST' | 'PUT' | 'PATCH',
  path: string,
  token: string | null,
  data?: unknown,
): Promise<T> {
  const res = await request.fetch(`${API}${path}`, {
    method,
    data,
    headers: token ? { Authorization: `Bearer ${token}` } : {},
  })
  const text = await res.text()
  if (!res.ok()) throw new Error(`${method} ${path} → ${res.status()} ${text}`)
  return (text ? JSON.parse(text) : null) as T
}

export async function register(
  request: APIRequestContext,
  name: string,
  prefix = 'user',
): Promise<Account> {
  const email = `${prefix}-${uniq()}@example.com`
  const r = await call<{ token: string; user: { id: string } }>(
    request,
    'POST',
    '/auth/register',
    null,
    { email, password: PASSWORD, name },
  )
  // 沒驗證 Email 不能開店:走真實流程,從信箱(Mailpit)取出驗證連結再點
  await verifyEmailViaMail(request, email)
  return { email, password: PASSWORD, name, id: r.user.id, token: r.token }
}

/** 信中驗證連結 `…/admin/verify#token=…` 的 token */
export function verifyTokenIn(mail: Mail): string {
  const link = linkIn(mail, '/admin/verify')
  return link.split('#token=')[1]
}

export async function verifyEmailViaMail(request: APIRequestContext, email: string) {
  const mail = await waitForMail(request, email, '驗證')
  await call(request, 'POST', '/auth/verify-email', null, { token: verifyTokenIn(mail) })
}

export interface Shop {
  slug: string
  name: string
  owner: Account
  serviceId: string
  serviceName: string
}

/** 開一家店:一位店主、一個 60 分鐘的服務、店主每天 09:00–18:00 提供該服務 */
export async function createShop(
  request: APIRequestContext,
  ownerName = '店主阿美',
): Promise<Shop> {
  const owner = await register(request, ownerName, 'owner')
  const slug = `e2e-${uniq()}`
  const name = `測試店 ${slug}`
  await call(request, 'POST', '/tenants', owner.token, { slug, name, timezone: 'Asia/Taipei' })
  const serviceName = '剪髮'
  const service = await call<{ id: string }>(request, 'POST', `/t/${slug}/services`, owner.token, {
    name: serviceName,
    duration_minutes: 60,
    price_cents: 50000,
  })
  await makeBookable(request, { slug, owner }, owner, service.id)
  return { slug, name, owner, serviceId: service.id, serviceName }
}

/** 讓某位成員每天 09:00–18:00 提供某服務 */
export async function makeBookable(
  request: APIRequestContext,
  shop: Pick<Shop, 'slug' | 'owner'>,
  member: Account,
  serviceId: string,
) {
  const hours = Array.from({ length: 7 }, (_, weekday) => ({
    weekday,
    start: '09:00',
    end: '18:00',
  }))
  await call(
    request,
    'PUT',
    `/t/${shop.slug}/members/${member.id}/working-hours`,
    shop.owner.token,
    {
      hours,
    },
  )
  await call(request, 'PUT', `/t/${shop.slug}/members/${member.id}/services`, shop.owner.token, {
    service_ids: [serviceId],
  })
}

/** 邀請一個已註冊的帳號成為員工並讓他接受 */
export async function addStaff(request: APIRequestContext, shop: Shop, name = '員工小安') {
  const staff = await register(request, name, 'staff')
  const inv = await call<{ token: string }>(
    request,
    'POST',
    `/t/${shop.slug}/invitations`,
    shop.owner.token,
    { email: staff.email, role: 'staff' },
  )
  await call(request, 'POST', '/invitations/accept', staff.token, { token: inv.token })
  return staff
}

/** 店主代客預約(直接成立) */
export async function bookAsOwner(
  request: APIRequestContext,
  shop: Shop,
  start: string,
  customer: { name: string; email: string; phone?: string },
  staffId?: string,
) {
  return call<{ id: string; manage_token: string; starts_at: string }>(
    request,
    'POST',
    `/t/${shop.slug}/bookings`,
    shop.owner.token,
    { service_id: shop.serviceId, staff_id: staffId ?? null, start, customer },
  )
}

// ---------- 信箱(Mailpit,真的走 SMTP) ----------

interface MailSummary {
  ID: string
  Subject: string
}

export interface Mail {
  subject: string
  text: string
}

/** 等到寄給 `to` 的信出現(標題含 `subject`),回傳內容。寄信由背景 worker 負責,每秒輪詢一次 */
export async function waitForMail(
  request: APIRequestContext,
  to: string,
  subject: string,
): Promise<Mail> {
  let found: MailSummary | undefined
  await expect
    .poll(
      async () => {
        const res = await request.get(`${MAILPIT}/api/v1/search`, {
          params: { query: `to:"${to}"` },
        })
        const body = (await res.json()) as { messages?: MailSummary[] }
        found = body.messages?.find((m) => m.Subject.includes(subject))
        return found?.Subject ?? null
      },
      { message: `等不到寄給 ${to} 的信(標題含「${subject}」)`, timeout: 30_000 },
    )
    .not.toBeNull()
  const res = await request.get(`${MAILPIT}/api/v1/message/${found!.ID}`)
  const msg = (await res.json()) as { Subject: string; Text: string }
  return { subject: msg.Subject, text: msg.Text }
}

/** 信中的第一個連結,換成測試用的網址格式(只取路徑與 # 之後) */
export function linkIn(mail: Mail, pathPrefix: string): string {
  const m = new RegExp(`https?://[^\\s]*?(${pathPrefix}[^\\s]*)`).exec(mail.text)
  if (!m) throw new Error(`信裡沒有 ${pathPrefix} 連結:\n${mail.text}`)
  return m[1]
}

/** 目前信箱裡寄給 `to` 的信有幾封(標題含 `subject`) */
export async function countMails(
  request: APIRequestContext,
  to: string,
  subject: string,
): Promise<number> {
  const res = await request.get(`${MAILPIT}/api/v1/search`, { params: { query: `to:"${to}"` } })
  const body = (await res.json()) as { messages?: MailSummary[] }
  return body.messages?.filter((m) => m.Subject.includes(subject)).length ?? 0
}

// ---------- 畫面 ----------

/** 填表送出登入。只負責送出 —— 登入失敗的測試要用它;成功的流程請用 `signIn` */
export async function loginViaUi(page: Page, email: string, password = PASSWORD) {
  await page.goto('/admin/login')
  await page.getByLabel('Email').fill(email)
  await page.getByLabel('密碼').fill(password)
  await page.getByRole('button', { name: '登入' }).click()
}

/**
 * 登入並等到真的登入完成(離開登入頁)。
 * 登入是非同步的:送出後馬上跳到別頁,會把還在進行中的登入請求打斷,cookie 就不會被設下來。
 */
export async function signIn(page: Page, email: string, password = PASSWORD) {
  await loginViaUi(page, email, password)
  await expect(page).not.toHaveURL(/\/admin\/login/)
}

/**
 * 全新的、沒有任何 cookie 的 API 請求環境。
 * 測試內建的 `request` 會記住 Set-Cookie(註冊時拿到的登入 cookie 會自動帶上),
 * 要驗證「沒登入」的行為必須用這個。
 */
export function anonymousApi(): Promise<APIRequestContext> {
  return playwrightRequest.newContext()
}
