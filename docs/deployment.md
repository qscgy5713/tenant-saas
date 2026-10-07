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

### 3b. 建立備份帳號(管理員,一次性)

```bash
psql "postgres://admin@DB_HOST/postgres" \
     -v backup_password="'換成第三組強密碼'" \
     -f deploy/provision_backup.sql
```

`tenant_backup`:`BYPASSRLS` + `pg_read_all_data`,**只能讀**、不是超級使用者。**不能用 migrator 備份**:
`customers`、`services` 等表是 FORCE RLS,連擁有者都受限,`pg_dump` 會直接失敗(見「備份與還原」)。
`BYPASSRLS` 只有超級使用者能授予;託管資料庫不允許時,改用服務商的管理員帳號備份。

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

**資料庫連線的 TLS**(已對「只接受 TLS 連線」的 PostgreSQL 實測,應用程式、migration、`pg_dump` 三者都可以):

| `sslmode` | 加密 | 驗證伺服器憑證 | 建議 |
|---|---|---|---|
| `require` | 是 | **否**(能防竊聽,不能防冒充的伺服器) | 資料庫在同一個私有網路時可接受 |
| `verify-full` + `sslrootcert=/路徑/ca.crt` | 是 | 是(簽發者與主機名稱) | **資料庫不在同一個私有網路時用這個**;CA 檔由託管服務商提供,掛進容器 |

實測:伺服器只收 TLS 時 `sslmode=disable` 被拒絕;`verify-full` 配錯的 CA 會拒絕連線(`UnknownIssuer`)。
資料庫端建議在 `pg_hba.conf` 只留 `hostssl`,讓沒加密的連線根本進不來。

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
| `AUDIT_RETENTION_DAYS` | | 730 | 稽核日誌保留天數,超過的由背景任務清除(每個店家會留一筆 `audit.purged` 收據)。`0` = 永久保留;其他值不得少於 90。清除前請確認需要的人已經匯出(稽核頁有 CSV 匯出) |
| `REQUIRE_VERIFIED_EMAIL` | | true | 沒驗證 Email 的使用者不能建立店家。只有明確寫 `false` / `0` 才關,**production 設為 false 會拒絕啟動**(只給開發與測試) |
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
- 網頁埠預設綁 `127.0.0.1:8080`,被占用時用 `WEB_PORT` 改(`PUBLIC_BASE_URL` 也要對應)。專案名稱是 `tenant-saas-prod`,和開發用的 compose 不會互相影響
- `/api/metrics` 在 nginx 直接回 404(指標只給內網的 Prometheus 向 `app:3001` 直接抓取)
- **已在本機完整演練過一次**(2026-10-06):建置兩個映像 → `migrate` → 以 production 模式 + 受限帳號啟動 → 健康檢查 → 經 nginx 註冊 / 驗證 Email(真實 SMTP 到 Mailpit)/ 開店 / CSRF 拒絕 → Prometheus 用 token 抓到指標、9 條規則載入且沒有誤報 → 真實瀏覽器走完整流程(`__Host-session` + HttpOnly + Secure + SameSite=Strict)→ `docker stop` 1 秒內優雅關閉 → 各種錯誤設定(超級使用者連線、缺 SMTP、關閉驗證、稽核保留過短、指標 token 過短)都在啟動時被拒絕。**尚未在真實主機與真實 HTTPS(憑證、反向代理、真實 SMTP 服務商)上驗證**

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

## 維運指令

營運人員用的命令列工具,是 `tenant-saas` 執行檔的子命令(映像裡本來就有,不需要另外裝東西、沒有新的網路攻擊面)。
用 **migrator 帳號**連線(`MIGRATION_DATABASE_URL`):runtime 帳號讀不到租戶資料表,這是設計上的。

```bash
A='docker compose -f deploy/docker-compose.prod.yml --env-file deploy/.env.prod --profile ops run --rm admin'
$A plans                          # 方案與限額
$A shops 20                       # 最新 20 家:方案、狀態、成員數、近 30 天預約數、預定刪除日
$A show my-salon                  # 一家店的詳細資料(店主、用量、訂閱、刪除狀態)
$A set-plan my-salon pro          # 變更方案(寫稽核 operator.plan_changed,店主在稽核頁看得到)
$A suspend my-salon               # 暫停:後台與預約頁都進不去,資料與訂閱不動(寫稽核)
$A unsuspend my-salon
$A export-audit 700 | gzip > audit-$(date +%F).jsonl.gz   # 稽核歸檔(見下)
```

- 所有會改資料的指令都寫稽核(actor = 系統,動作以 `operator.` 開頭)。店家代稱先驗證格式才查詢。
- 標準輸出只有資料(日誌與摘要走標準錯誤),所以可以直接接 `gzip`。

**稽核日誌歸檔**:背景任務會依 `AUDIT_RETENTION_DAYS`(預設 730)刪除過期的稽核日誌。要保留舊紀錄,在它動手**之前**
定期把「快要過期」的匯出(例如每週一次,匯出超過 `保留天數 − 30` 天的):

```bash
# crontab:每週一 03:30,保留 730 天 → 歸檔超過 700 天的(和已歸檔的重疊沒關係,id 可用來去重)
30 3 * * 1  $A export-audit 700 | gzip > /var/backups/tenant-saas/audit-$(date +\%F).jsonl.gz
```

格式是 JSON Lines(一行一筆,依 id 由舊到新,含店家代稱):**不用 CSV**,因為 CSV 為了防試算表公式注入會改動欄位開頭,
歸檔要原樣保存。歸檔檔含所有店家的操作紀錄,請設嚴格的檔案權限並放在資料庫主機以外的地方。

## 映像與依賴的安全掃描

CI 每次推送與**每週一**(`schedule`:漏洞資料庫每天更新,程式沒動也可能出現新漏洞)都會:
- 用 Trivy 掃兩個映像與 `Cargo.lock` / `package-lock.json`;有「已有修正版」的 HIGH / CRITICAL 漏洞就失敗(沒有修正版的無從處理,不擋)
- 產生 CycloneDX 格式的 SBOM,存成 CI 產物(`sbom-api`、`sbom-web`,保留 90 天)
- Dockerfile 在執行階段先升級基底映像已有的套件(`apt-get upgrade` / `apk upgrade`):`nginx:1.27-alpine` 是釘住的次版本,不升級的話本機掃出 44 個 HIGH / CRITICAL,升級後為 0
- **映像簽章沒做**:需要先決定映像放在哪個 registry(簽章綁定映像的摘要)

本機執行:`docker run --rm -v /var/run/docker.sock:/var/run/docker.sock aquasec/trivy:0.75.0 image --severity HIGH,CRITICAL --ignore-unfixed 映像名稱`

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

## 資料保留與刪除

背景任務每小時執行一次(`WORKER_ENABLED=true` 才會跑),**一次最多處理 200 位顧客**,量大時下一輪接著做:

| 對象 | 規則 | 設定 |
|---|---|---|
| 顧客個資 | 最後一筆預約之後超過保留天數,且沒有進行中的預約 → 匿名化(姓名、Email、電話、預約備註抹除,預約與統計保留;未寄出的信刪除、已寄出的信件紀錄改匿名地址)。沒有預約的顧客以建立時間起算 | 每家店 90–3650 天,預設 730,店主在「設定」調整 |
| 稽核日誌 | 超過 `AUDIT_RETENTION_DAYS` 天 → 刪除 | 全系統,預設 730 |
| 信件佇列 | 已完成的 30 天後刪除 | 固定 |
| 未驗證的帳號 | 註冊後 30 天仍沒驗證 Email、也沒加入任何店家 → 刪除(釋出被占住的 Email) | 固定 |
| 帳號安全事件 | 180 天後刪除(使用者在店家列表頁看得到自己的「最近的帳號活動」) | 固定 |
| 整家店 | 店主申請 → 30 天寬限(公開預約頁與預約管理連結立即關閉、不能新增預約、可取消)→ 期滿連同所有資料刪除 | 寬限期固定 30 天 |

**要知道的限制:**
- **備份不會被清理。** 匿名化與刪除只作用在線上資料庫;已存在的備份裡仍有舊資料。請依你的保留政策為備份設定到期時間,並在處理「刪除我的資料」的請求時把備份算進去。
- 刪除店家不會刪除**使用者帳號**(同一個帳號可能屬於其他店家)。使用者可以自己刪除帳號(店家列表頁 →「刪除我的帳號」,要輸入密碼):帳號會被匿名化(Email 換成無法還原的代號並釋出、名稱與密碼抹除、所有登入失效、成員資格停用、寄給他的信與邀請上的 Email 一併處理);**還是店主、或還有負責的未來預約時不能刪**(店家已申請刪除的不算)。歷史預約與稽核保留,只是不再顯示他的名字。
- 日誌(JSON log)只記 id,不含個資;但已寫出的日誌不會被回頭清理。
- 申請刪除的前提:沒有未來或待確認的預約、沒有進行中的訂閱。
- 監控:`customers_anonymized_total`、`audit_rows_purged_total`、`tenants_deleted_total`。`tenants_deleted_total` 增加是不可逆的事件,建議接通知。

## 備份與還原

`deploy/backup.sh`(cron 每天一次):`pg_dump` custom 格式 → 先寫暫存檔並用 `pg_restore --list` 驗證能讀 → 才改名成正式檔案(不會留下半份備份,權限 600)→ 清理超過 `BACKUP_RETENTION_DAYS`(預設 35)天的舊備份,**但永遠保留最新 `BACKUP_KEEP_MIN`(預設 3)份**,備份壞掉一陣子時不會把僅存的備份清光。`deploy/backup_test.sh` 測試清理與驗證邏輯(CI 會跑)。

```bash
BACKUP_DATABASE_URL='postgres://tenant_backup:密碼@DB_HOST/tenant_saas?sslmode=require' \
BACKUP_DIR=/var/backups/tenant-saas  deploy/backup.sh
```

- **用備份帳號 `tenant_backup`**(「第一次部署」步驟 3b)。**不能用 migrator**:FORCE RLS 的表連擁有者都受限,`pg_dump` 會直接失敗;加上 `--enable-row-security` 則會「成功」但那些表是空的。**不要用 runtime 帳號**(它讀不到租戶資料表)。
- **確認備份是完整的**:還原到另一個資料庫,比對各表筆數(CI 的 `restricted-role` 工作每次都這樣做:用 `tenant_backup` 備份 → 還原 → 逐表比對,包含 `customers`)。換了備份帳號或資料庫服務商之後,至少手動做一次。備份檔要放在**不是資料庫主機**的地方,並且另外複製到異地。
- **角色是整個叢集共用的,不在資料庫備份裡**:另外用 `pg_dumpall --roles-only` 備份,或還原到新叢集時先照「第一次部署」的步驟 1–3 建立 migrator 與角色。
- 已用真實的 `pg_dump` / `pg_restore` 演練過備份與還原(還原後各表筆數與來源一致)。**注意**:早期的演練是用超級使用者做的,所以沒發現 migrator 備份不了;現在的演練與 CI 都用 `tenant_backup`。**還原到新叢集後還要重新啟用 runtime 帳號的登入與角色授權**(同「第一次部署」步驟 3)。
- **`BACKUP_RETENTION_DAYS` 就是「刪除資料的請求真正從系統消失」的最長期限**:匿名化與刪除只作用在線上資料庫,備份裡的資料不會變。請依你的隱私政策決定天數,並寫進隱私權說明。
- **還原會讓已刪除的資料復活。** 從備份還原之後,備份時間點之後做的匿名化(顧客資料刪除、帳號刪除)會消失。應用程式日誌會記下每次刪除的 id(`顧客資料已刪除(匿名化)`、`使用者帳號已刪除(匿名化)`、`依保留政策匿名化顧客`,只有 id 沒有個資):還原後請依日誌把這些重做一次,**日誌也要保存得比備份久**。依保留政策到期的資料會由背景任務自動再處理一次。
- 稽核日誌(`audit_logs`):一般角色只能新增;只有保留政策的清理函式能刪過期的(見「資料保留與刪除」)。
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
- [x] 營運人員:**命令列維運指令**(`tenant-saas admin …`,見「維運指令」),不做網頁後台(決定理由見 decisions)
- [x] 帳號鎖定(5 次 / 15 分鐘);告警建議:`account_locked_total` 突然升高代表有人在猜密碼
- [x] 註冊 Email 驗證(沒驗證不能建立店家)、登出所有裝置。**不做 refresh token**:每個請求本來就查資料庫,改密碼 / 登出所有裝置都會立刻讓舊 token 失效(見 decisions)。缺的是「單一裝置登出」與「session 清單」
- [x] 資料保留與刪除(顧客個資自動匿名化、稽核日誌到期清除、刪除店家);使用者帳號刪除、備份腳本與到期清理也已完成;稽核日誌可在清除前用 `admin export-audit` 歸檔;**尚缺**:備份的異地複製與加密(腳本只負責產生與清理)、排程(cron)要自己設
- [x] 資料庫連線 TLS:`sslmode=require` / `verify-full` 已對只收 TLS 的 PostgreSQL 實測(應用程式、migration、備份)
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
