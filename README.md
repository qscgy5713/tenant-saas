# Tenant SaaS

以 Rust(axum + sqlx + PostgreSQL)實作的多租戶 SaaS 後端。

## 文件
- [plan.md](docs/plan.md):專案計畫與架構
- [todo.md](docs/todo.md):待辦清單
- [worklog.md](docs/worklog.md):工作日誌
- [decisions.md](docs/decisions.md):設計決策紀錄
- [schema.md](docs/schema.md):資料表與 RLS 設計
- [deployment.md](docs/deployment.md):**部署指南**(資料庫角色、步驟、環境變數、監控、上線前檢查表)

## 狀態
M1–M9 全部完成(骨架、認證、多租戶隔離、服務與排程設定、邀請與成員管理、預約核心、Email 確認與背景寄信、方案與限額、稽核日誌與可觀測性、Docker 與部署)。**有顧客預約頁與店家後台(`web/`);已串接 Stripe 訂閱(結帳 / 客戶入口 / webhook),但只對 `stripe-mock` 與本機簽章測試驗證過,尚未用真實 Stripe 帳號驗證。**未設定 Stripe 時方案只能由營運人員改資料庫。 上線前請讀 [docs/deployment.md](docs/deployment.md) 的檢查表。

## 快速開始
```
cp .env.example .env
docker compose up -d postgres redis
cargo run
curl 127.0.0.1:3001/health
```

## 前端
`web/` 是前端(React + TypeScript + Vite),分兩部分:
- **顧客預約頁**(`/s/:slug`):選服務、選日期時間、Email 確認、管理預約(改期 / 取消)。
- **店家後台**(`/admin`):登入 / 註冊、建店、預約(依日期 / 待確認 / 近 30 天、代客預約、取消、完成 / 未到、備註)、服務、團隊(邀請、角色、移除)、成員的營業時間 / 可提供的服務 / 休假、稽核日誌、方案與用量。後台是獨立的程式碼 chunk,顧客不會下載。

開發方式見 [web/README.md](web/README.md)。示範資料:`python3 scripts/seed_demo.py --with-bookings`,顧客頁 `/s/demo-salon`,後台 `/admin`(`owner@demo.example.com` / `demo-password-123`)。

## 目前的 API
| 方法 | 路徑 | 說明 |
|---|---|---|
| GET | `/health` | 健康檢查(含資料庫) |
| POST | `/auth/register`、`/auth/login` | 註冊、登入,回傳 JWT |
| GET | `/auth/me` | 目前使用者 |
| POST / GET | `/tenants` | 建立店家 / 列出我所屬的店家 |
| GET | `/t/{slug}/me` | 我在該店的角色 |
| GET | `/t/{slug}/members` | 該店成員 |
| GET | `/t/{slug}/plan` | 目前方案與用量(manager 以上) |
| GET | `/plans` | 公開的方案列表 |
| GET | `/t/{slug}/audit-logs` | 稽核日誌(manager 以上,唯讀):`?action=booking.&entity_type=&entity_id=&actor_user_id=&from=&to=&limit=&before=` |
| GET | `/metrics` | Prometheus 指標,需 `Authorization: Bearer $METRICS_TOKEN`;未設定 `METRICS_TOKEN` 時不存在 |
| GET / POST | `/t/{slug}/services` | 服務項目清單(`?active=&limit=&offset=`)/ 新增(manager) |
| GET / PATCH / DELETE | `/t/{slug}/services/{id}` | 查詢 / 局部更新 / 刪除(manager) |
| GET / PUT | `/t/{slug}/members/{user_id}/services` | 員工可提供的服務(PUT 為 manager) |
| GET / PUT | `/t/{slug}/members/{user_id}/working-hours` | 每週營業時間(PUT 為 manager 或本人),weekday 0 = 週日 |
| GET / POST | `/t/{slug}/members/{user_id}/time-off` | 休假(POST 為 manager 或本人) |
| DELETE | `/t/{slug}/time-off/{id}` | 刪除休假(manager 或本人) |
| GET / POST | `/t/{slug}/invitations` | 待處理邀請 / 建立邀請(回傳一次性 token) |
| DELETE | `/t/{slug}/invitations/{id}` | 撤銷邀請 |
| POST | `/invitations/accept` | 被邀請者以 token 加入店家(需登入且 Email 相符) |
| PATCH | `/t/{slug}/members/{user_id}` | 改角色(僅 owner) |
| DELETE | `/t/{slug}/members/{user_id}` | 移除成員 / 自己退出(owner 不可被移除) |

### 公開預約(顧客,不需登入;有限流)
| 方法 | 路徑 | 說明 |
|---|---|---|
| GET | `/public/shops/{slug}` | 店家名稱與時區 |
| GET | `/public/shops/{slug}/services` | 可預約的服務 |
| GET | `/public/shops/{slug}/services/{id}/staff` | 提供該服務的員工 |
| GET | `/public/shops/{slug}/availability?service_id=&from=&to=&staff_id=` | 可預約時段(日期為店家本地日期,一次最多 14 天) |
| POST | `/public/shops/{slug}/bookings` | 提出預約申請(狀態 `pending`,**不占時段**),系統寄確認信;回應不含 token |
| POST | `/public/bookings/{token}/confirm` | 按下信中連結後確認,此刻才占位 |
| GET | `/public/bookings/{token}` | 查看預約(token 只寄到顧客信箱) |
| GET | `/public/bookings/{token}/availability?from=&to=` | 這筆預約改期時可選的時段(同一位員工與服務,**排除預約自己**) |
| POST | `/public/bookings/{token}/cancel`、`/reschedule` | 取消 / 改期 |

### 帳號補充(有限流)
| 方法 | 路徑 | 說明 |
|---|---|---|
| POST | `/auth/forgot-password` | 申請重設密碼。**不論 Email 是否註冊都回同樣的 202**;同一帳號每小時最多 3 封 |
| POST | `/auth/reset-password` | 用信中的 token 設定新密碼(單次、1 小時有效);成功後該帳號所有舊登入立即失效 |

### 計費(Stripe;需設定 `STRIPE_*`,否則回 501)
| 方法 | 路徑 | 說明 |
|---|---|---|
| GET | `/t/{slug}/billing` | 訂閱狀態與可訂閱方案(僅店主) |
| POST | `/t/{slug}/billing/checkout` | `{plan}` → Stripe 結帳頁網址;已有進行中的訂閱回 409 |
| POST | `/t/{slug}/billing/portal` | Stripe 客戶入口網址(變更方案、付款方式、取消) |
| POST | `/webhooks/stripe` | Stripe 呼叫;驗證簽章(原始 body、5 分鐘容忍度),事件去重、忽略亂序的舊事件。**方案只由 webhook 改變** |

### 員工端預約(需登入)
| 方法 | 路徑 | 說明 |
|---|---|---|
| GET / POST | `/t/{slug}/bookings` | 列表(staff 只看自己的)/ 代客預約 |
| PATCH | `/t/{slug}/bookings/{id}` | 改狀態(cancelled / completed / no_show)與備註 |

## 稽核與可觀測性
- **稽核日誌**:誰在何時改了什麼,與變更同一交易寫入,資料庫層面只能新增、不能改或刪。內容不含 Email、電話、token、休假原因。
- **日誌**:預設人類可讀;`LOG_FORMAT=json` 輸出 JSON。每筆請求日誌帶 `request_id` 與路由樣板(`/public/bookings/{token}`),**不記實際網址**,因為網址裡有顧客的預約管理 token。
- **請求 ID**:沿用合法的 `X-Request-Id`(英數、`-`、`_`,≤ 64 字),否則產生新的,回應一律帶回。
- **指標**:設定 `METRICS_TOKEN`(至少 16 字元)後開放 `GET /metrics`,含請求數 / 延遲、寄信與 worker 計數,以及 `email_outbox_pending`、`email_outbox_failed`、`email_outbox_oldest_due_age_seconds`(信件積壓多久)。

## 方案與限額
新店家預設「免費版」:2 位成員(含待接受的邀請)、5 項啟用中的服務、每月 50 筆預約(依店家當地月份)。超過時員工端回 **402**,顧客端一律回 409「額滿」(不暴露方案資訊)。上限依 `plans` 表,數字僅為示範。

## 寄信
- 設定 `SMTP_URL`(例如 `smtps://user:pass@smtp.example.com:465`)與 `MAIL_FROM`,背景 worker 會寄出確認信、提醒信、邀請信。
- 沒設 `SMTP_URL` 時是開發模式:信件(含連結)只印在日誌,不會真的寄出。
- 本機收信測試:`docker compose up -d mailpit`,設 `SMTP_URL=smtp://127.0.0.1:1025`,信件看 http://127.0.0.1:8025。
- `PUBLIC_BASE_URL` 是信中連結的前端網址;`WORKER_ENABLED=false` 可關閉 worker。

## 測試
```
docker compose up -d postgres
cargo test            # 一般測試,用超級使用者連線
```
一般測試證明不了「正式環境用受限帳號時一切正常」。另有一個預設忽略的測試用受限帳號跑完整流程,
需要乾淨的 PostgreSQL 叢集,步驟見 `docs/deployment.md` 末段與 `.github/workflows/ci.yml`。

## Docker
```
docker build -t tenant-saas .
docker run --rm tenant-saas migrate   # 一次性,用 MIGRATION_DATABASE_URL
docker run -d -p 3001:3001 tenant-saas # 預設 APP_ENV=production:資料庫帳號過大會拒絕啟動
```
**不要在正式環境用超級使用者連線**——會繞過租戶隔離。詳見 `docs/deployment.md`。
