# 部署指南

## 最重要的一件事:資料庫帳號

租戶隔離靠 PostgreSQL 的 RLS。**超級使用者、BYPASSRLS 角色、資料表擁有者都會繞過它。**
所以應用程式絕不能用這些身分連線。這個專案用「四個角色」把權限切開:

| 角色 | 用途 | 能登入 | 備註 |
|---|---|---|---|
| 管理員(`postgres` 等) | 一次性佈建 | 是 | 只在第一次建立 migrator 與資料庫時用 |
| `tenant_migrator` | 套用 migration | 是 | 資料庫與資料表的擁有者,有 `CREATEROLE`,**不是**超級使用者 |
| `tenant_runtime` | **應用程式連線帳號** | 是(你啟用) | `NOINHERIT`;只直接讀寫 `users`、讀 `plans`,**碰不到任何租戶資料表** |
| `tenant_app` / `tenant_worker` | 請求 / 背景任務執行時 `SET LOCAL ROLE` 切過去 | 否 | 受 RLS 約束,只有必要的權限 |

每個請求都先 `SET LOCAL ROLE tenant_app` 才碰租戶資料。因此如果哪天有人漏了經過 `begin_scoped`、直接對連線池查詢,
會得到 `permission denied`(失敗即關閉),而不是悄悄讀到別家店的資料。

`APP_ENV=production` 時,程式啟動會檢查連線帳號,**過大就拒絕啟動**:

```
Error: APP_ENV=production 拒絕啟動,資料庫連線帳號不夠受限:
  - 連線帳號是超級使用者,會繞過所有 RLS 與權限檢查
  - 連線帳號可以直接讀取租戶資料表:tenants、memberships、…
```

需要 **PostgreSQL 16 以上**(使用 `GRANT … WITH INHERIT FALSE, SET TRUE`)。

## 第一次部署

以下指令都在演練時實際執行過(PostgreSQL 17、映像 `tenant-saas`)。

### 1. 佈建 migrator 與資料庫(管理員,一次性)

```bash
psql "postgres://admin@DB_HOST/postgres" \
     -v migrator_password="'換成強密碼'" \
     -f deploy/provision.sql
```

建立 `tenant_migrator` 與由它擁有的資料庫 `tenant_saas`。`citext`、`btree_gist` 是 PostgreSQL 的「受信任擴充」,
資料庫擁有者就能建立,**不需要超級使用者**。

### 2. 套用 migration(以 migrator)

```bash
docker run --rm \
  -e MIGRATION_DATABASE_URL='postgres://tenant_migrator:密碼@DB_HOST/tenant_saas' \
  tenant-saas:VERSION migrate
```

這會建立資料表、`tenant_app`、`tenant_worker`、`tenant_runtime` 三個角色(後者 `NOLOGIN`)。

### 3. 啟用 runtime 帳號的登入

migration 不保管密碼,所以由你設定:

```bash
psql "postgres://tenant_migrator:密碼@DB_HOST/tenant_saas" \
     -c "ALTER ROLE tenant_runtime WITH LOGIN PASSWORD '換成另一組強密碼'"
```

### 4. 啟動應用程式(以 runtime)

```bash
docker run -d --name tenant-saas -p 127.0.0.1:3001:3001 \
  -e DATABASE_URL='postgres://tenant_runtime:密碼@DB_HOST/tenant_saas?sslmode=require' \
  -e JWT_SECRET="$(openssl rand -hex 32)" \
  -e SMTP_URL='smtps://帳號:密碼@smtp.example.com:465' \
  -e MAIL_FROM='預約系統 <noreply@example.com>' \
  -e PUBLIC_BASE_URL='https://book.example.com' \
  -e METRICS_TOKEN="$(openssl rand -hex 16)" \
  tenant-saas:VERSION
```

映像預設 `APP_ENV=production`、`LOG_FORMAT=json`、以非 root(uid 10001)執行,內建 `HEALTHCHECK`。
production 模式**預設不在啟動時套用 migration**(執行階段帳號本來就沒有建表權限)。

## 升級

1. 建置新映像。
2. **先**用 `migrate` 一次性容器套用新的 migration(步驟 2)。
3. 再滾動更新應用程式容器。

Migration 只往前、不回頭。要讓新舊版本能在滾動更新期間並存,migration 應只做「加法」(新增欄位 / 資料表),
刪除或改名要拆成兩次發版。

> **陷阱**:`sqlx::migrate!` 在**編譯時**嵌入 migration。專案已有 `build.rs` 讓 Cargo 追蹤 `migrations/`,
> 否則單獨新增一個 migration 檔,程式會回報「migration 完成」卻沒套用新檔。升級後可用
> `SELECT max(version) FROM _sqlx_migrations` 核對。

## 環境變數

| 變數 | 必填 | 預設 | 說明 |
|---|---|---|---|
| `DATABASE_URL` | 是 | — | **runtime 帳號**的連線字串(正式環境加 `?sslmode=require`) |
| `MIGRATION_DATABASE_URL` | 僅 `migrate` | 退回 `DATABASE_URL` | migrator 帳號 |
| `JWT_SECRET` | 是 | — | 至少 32 字元;更換會讓所有已簽發的 token 失效 |
| `JWT_TTL_SECS` | | 3600 | token 有效秒數 |
| `APP_ENV` | | 映像內為 `production` | `production`:帳號過大拒絕啟動、必須設 `SMTP_URL`、預設不自動 migrate |
| `AUTO_MIGRATE` | | 開發 true / production false | 啟動時是否套用 migration |
| `BIND_ADDR` | | `127.0.0.1:3001`(映像內 `0.0.0.0:3001`) | |
| `SMTP_URL` | production 必填 | — | 例如 `smtps://user:pass@host:465`。**未設定時信件(含一次性連結)會被印進日誌**,所以 production 會拒絕啟動 |
| `MAIL_FROM` | | `預約系統 <noreply@localhost>` | |
| `PUBLIC_BASE_URL` | | `http://localhost:3001` | 信中連結的前端網址;**同時是允許以 cookie 登入並送出寫入請求的網頁來源**(CSRF 的 Origin 白名單) |
| `ALLOWED_ORIGINS` | | (空) | 額外允許的網頁來源,逗號分隔,例如 `https://admin.example.com`。非正式環境另外自動允許 `http://localhost:5173` |
| `WORKER_ENABLED` | | true | 背景寄信 / 提醒 / 整理過期預約 |
| `WORKER_POLL_SECS` | | 5 | |
| `PUBLIC_RATE_LIMIT_PER_MIN` | | 60 | 公開預約端點,每來源 |
| `AUTH_RATE_LIMIT_PER_MIN` | | 10 | 註冊 / 登入,每來源 |
| `MAX_MAILS_PER_RECIPIENT_PER_HOUR` | | 6 | 每位收件者每小時最多收幾封信(跨店家;防止拿別人的 Email 當收件者騷擾)。只套用在寄給外部指定收件者的信 |
| `MAX_SHOPS_PER_USER` | | 5 | 每位使用者最多能「擁有」幾家店(防灌店家 / 占用代稱);受邀加入別人的店不算;0 視為設定錯誤 |
| `TRUST_PROXY` | | false | 見下方「反向代理」 |
| `STRIPE_SECRET_KEY` | | 空 = 不啟用 | 與 `STRIPE_WEBHOOK_SECRET` 必須同時設定,且至少一個 `STRIPE_PRICE_*`,否則拒絕啟動 |
| `STRIPE_WEBHOOK_SECRET` | | — | Stripe Dashboard 的 webhook 簽署密鑰(`whsec_…`);webhook 網址 `https://<api>/webhooks/stripe`,訂閱事件 `customer.subscription.created/updated/deleted` 、`invoice.payment_failed`(付款失敗通知信)與 `invoice.paid`(付款恢復通知信;沒訂閱這兩個事件就不會寄) |
| `STRIPE_PRICE_PRO`、`STRIPE_PRICE_BUSINESS` | | — | 對應方案的 Stripe Price id;沒設的方案不能線上訂閱 |
| `STRIPE_API_BASE` | | `https://api.stripe.com` | 測試時指向 `stripe-mock` |
| `METRICS_TOKEN` | | 空 = 關閉 | ≥ 16 字元;設定後才有 `GET /metrics` |
| `LOG_FORMAT` | | 映像內 `json` | `json` 或留空 |
| `RUST_LOG` | | `info` | |

## Docker Compose 部署範例

`deploy/docker-compose.prod.yml`:`app` + `web`(nginx)+ 一次性的 `migrate`(profile)+ 選用的 `prometheus`(profile `monitoring`,已掛上告警規則)。
**不含資料庫** —— 請用託管的 PostgreSQL,並先照「第一次部署」佈建帳號。

```bash
cp deploy/env.prod.example deploy/.env.prod        # 填值;.gitignore 已排除
docker compose -f deploy/docker-compose.prod.yml --env-file deploy/.env.prod --profile migrate run --rm migrate
docker compose -f deploy/docker-compose.prod.yml --env-file deploy/.env.prod up -d
```

- `app` 不對外開放埠,只有 `web` 與 `prometheus` 能連到;`web` 綁 `127.0.0.1:8080`,TLS 由前面的負載平衡器終止
- 因為前面是自己的 nginx,compose 裡設了 `TRUST_PROXY=true`(見下方「反向代理與 TLS」);若你在 `web` 前面又加一層代理,要確認每一層都是「附加」而不是「轉傳」
- 必填的變數沒填會直接報錯(`${VAR:?}`),不會悄悄用空值啟動
- 要監控:建立 `deploy/secrets/metrics_token`(內容與 `METRICS_TOKEN` 相同),加 `--profile monitoring`
- **這份範例只驗證過 `docker compose config` 能解析、規則檔能通過 promtool;沒有在真實主機上跑過一次完整部署**

## 反向代理與 TLS

應用程式本身不處理 TLS,請放在 nginx / Caddy / 雲端負載平衡器後面終止 TLS。

- 限流用「來源 IP」。**預設不信任 `X-Forwarded-For`**(否則任何人都能偽造來源繞過限流)。
- 部署在**你控制的**反向代理後面時才設 `TRUST_PROXY=true`;此時取 `X-Forwarded-For` **最右邊**那一筆
  (最右是你的代理附加的,左邊的可由用戶端偽造)。代理必須**附加**而不是**轉傳**用戶端帶來的標頭。
- 如果代理後面沒設 `TRUST_PROXY`,所有使用者會共用代理的 IP,一起被限流。
- 限流是**單一實例記憶體內**的:跑多個實例時各自計算,實際上限是「設定值 × 實例數」。要全域限流需改用 Redis。
- 不要把 `/metrics` 開放到公網:即使有 token,也建議只讓內網的 Prometheus 抓取。

## 前端

`web/` 是 React 單頁應用程式,正式環境用 `web/Dockerfile`(nginx 提供靜態檔 + 把 `/api/` 轉給後端並去掉前綴):

```bash
docker build -t tenant-saas-web web
docker run -d -p 80:80 -e API_UPSTREAM=http://app:3001 tenant-saas-web
```

後端對應設定:
- **登入用 HttpOnly cookie**(正式環境 cookie 名稱 `__Host-session`,`Secure; HttpOnly; SameSite=Strict; Path=/`)。因此:
  - **API 必須與網頁同源**(nginx 的 `/api/` 代理已是);跨網域的 API 收不到 cookie
  - 一定要走 **HTTPS**:`__Host-` 與 `Secure` 的 cookie 在 http 下瀏覽器會直接丟掉,登入會「成功但沒登入」
  - 反向代理要原樣轉送 `Origin` 標頭(nginx 預設就會)與 `Set-Cookie`;若有自訂 `proxy_set_header Origin` 或 `proxy_hide_header Set-Cookie` 要拿掉
  - 用 cookie 認證的 POST / PUT / PATCH / DELETE 會檢查 `Origin` 是否在白名單(`PUBLIC_BASE_URL` + `ALLOWED_ORIGINS`);網頁來源和 `PUBLIC_BASE_URL` 不同(例如另有後台網域)就要加進 `ALLOWED_ORIGINS`,否則寫入請求會 403
  - Bearer token(`Authorization` 標頭)仍可用,給腳本 / 測試;不檢查 Origin
- `PUBLIC_BASE_URL` 設成**前端的公開網址**(信中的連結是 `${PUBLIC_BASE_URL}/bookings/{token}`)。
- 後端在 nginx 後面時設 `TRUST_PROXY=true`,限流才會用真實的客戶端 IP(nginx 設定用 `$proxy_add_x_forwarded_for` 附加,後端取最右邊那筆)。
- 前端與 API 在同一個網域(`/api/` 代理),所以不需要 CORS。

nginx 設定(`web/nginx.conf.template`)刻意做的事,因為**預約管理頁的網址裡有 token**:
- `Referrer-Policy: no-referrer`:瀏覽器不會把含 token 的網址當 Referer 送給別的網站
- `/bookings/*` 加 `X-Robots-Tag: noindex` 與 `Cache-Control: no-store`
- 嚴格的 CSP(只允許同源的腳本與樣式)、`frame-ancestors 'none'`(不能被嵌進別人的頁面)
- 帶 hash 的 `/assets/` 永久快取;不存在的資產回 404(不被 SPA 的 fallback 吃掉)

## 監控

```yaml
# prometheus.yml
scrape_configs:
  - job_name: tenant-saas
    metrics_path: /metrics
    authorization: { type: Bearer, credentials: "METRICS_TOKEN 的值" }
    static_configs: [{ targets: ["app:3001"] }]
```

告警規則檔:`deploy/alerts.yml`(9 條,`deploy/alerts_test.yml` 用 `promtool test rules` 驗證過會在該響的時候響)。
`deploy/prometheus.yml` 是搭配的抓取設定。驗證:

```bash
docker run --rm --entrypoint promtool -v "$PWD/deploy:/d" prom/prometheus:latest check rules /d/alerts.yml
docker run --rm --entrypoint promtool -v "$PWD/deploy:/d" prom/prometheus:latest test rules /d/alerts_test.yml
```

規則涵蓋的條件(門檻是起點,請依實際流量調整):

| 條件 | 意義 |
|---|---|
| `app_db_up == 0` | 資料庫連不上 |
| `email_outbox_oldest_due_age_seconds > 300` | 信件積壓:SMTP 壞了或 worker 停了 |
| `increase(emails_failed_total[1h]) > 0` | 有信件最終寄送失敗 |
| `email_outbox_failed > 0` | 同上(保留期 30 天內的累計) |
| `rate(http_requests_total{status=~"5.."}[5m]) / rate(http_requests_total[5m]) > 0.01` | 5xx 比例 |
| `histogram_quantile(0.99, rate(http_request_duration_seconds_bucket[5m])) > 1` | P99 延遲 |

日誌是 JSON,每筆請求帶 `request_id` 與路由樣板(**不含實際網址**,因為網址裡有顧客的預約管理 token)。
`X-Request-Id` 會沿用上游帶來的合法值,方便串起整條請求。

## 備份與還原

- 用 `pg_dump` 備份資料庫。**角色是整個叢集共用的,不在資料庫備份裡**:還原到新叢集時,先照「第一次部署」的步驟 1–3
  建立 migrator 與角色,或用 `pg_dumpall --roles-only` 另外備份角色。
- 稽核日誌(`audit_logs`)只能新增;資料庫層面沒有任何應用角色能改或刪。目前**永久保留**,量大時需要歸檔(尚未實作)。
- `email_outbox` 內含待寄信件(寄出後清空內容);已完成的紀錄 30 天後自動清理。

## 上線前檢查表

已完成並有測試:

- [x] 租戶隔離(RLS)、受限的資料庫帳號、啟動時檢查
- [x] 密碼 argon2 雜湊;登入 / 註冊限流;公開預約端點限流
- [x] 預約管理 token 與邀請 token 只存雜湊,不進日誌與指標
- [x] 顧客 Email 確認後才占位;併發下不超賣、不重複預約
- [x] 稽核日誌(只增不減、不含個資)

**上線前請評估(已完成的打勾,沒勾的是尚未做):**

- [x] **Stripe / 計費**:結帳、客戶入口、webhook、付款失敗通知已實作;**尚未用真實 Stripe 帳號驗證**。沒設定 `STRIPE_*` 時方案仍只能由營運人員改資料庫
- [ ] 營運人員後台 / 平台超級管理員(跨租戶管理)
- [x] 帳號鎖定(5 次 / 15 分鐘);告警建議:`account_locked_total` 突然升高代表有人在猜密碼
- [ ] 註冊 Email 驗證、refresh token(JWT 目前只有單一短效 token,無法主動撤銷)
- [ ] 稽核日誌歸檔、資料保留與刪除政策(個資法 / GDPR 的刪除請求流程)
- [ ] 全域(跨實例)限流
- [x] 告警規則檔(`deploy/alerts.yml`)
- [ ] 分散式追蹤
- [x] 前端:顧客預約頁與店家後台(`web/`,nginx 映像)
- [ ] 負載測試;目前每家店每種限額資源用一把 advisory lock,寫入量極大的單一店家會在該資源上排隊

## 驗證部署本身

一般測試(`cargo test`)用超級使用者連線,**證明不了正式環境的行為**。另有一個測試專門用受限帳號把整個流程跑一遍,
包含「直接對連線池查租戶資料表必須被拒絕」與所有 SECURITY DEFINER 函式:

```bash
RESTRICTED_RUNTIME_URL='postgres://tenant_runtime:密碼@localhost:5432/tenant_saas' \
  cargo test --test restricted_role -- --ignored
```

CI 的 `restricted-role` 工作會在**全新的 PostgreSQL 叢集**上依上面的步驟 1–3 佈建後執行它,
並且在跑測試前撤銷 migrator 的角色成員資格,確保 SECURITY DEFINER 函式不依賴意外的權限。
