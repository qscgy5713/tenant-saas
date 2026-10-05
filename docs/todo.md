# Todo

狀態:`[ ]` 未開始、`[~]` 進行中、`[x]` 完成

## 前置
- [x] 確認領域:預約排程
- [x] 確認本機已安裝 Rust toolchain(1.99.0),Postgres、Redis 走 Docker
- [ ] 確認專案資料夾名稱

## M1 專案骨架
- [x] `cargo init`(單一 crate,暫不拆 workspace)
- [x] 設定 axum + tokio + tracing
- [x] 設定 sqlx + migration 目錄
- [x] 健康檢查 endpoint
- [x] 設定檔載入(環境變數 / config)
- [x] docker-compose(Postgres、Redis)

## M2 認證
- [x] users 資料表與 migration
- [x] 註冊 API
- [x] 登入 API(argon2 驗證)
- [x] JWT 機制(只放 user id,租戶每個請求另外驗證)
- [ ] 登入限流(防暴力破解)
- [ ] 評估把 users 查詢包進 SECURITY DEFINER 函式(見 schema.md 已知限制)
- [x] 認證 extractor

## M3 多租戶核心
- [x] 確認 `schema.md` 的設計與 6 項決定(照預設)
- [x] tenants、memberships 資料表
- [x] 租戶識別 middleware
- [x] 啟用 RLS 與 policy
- [x] 交易內 `SET LOCAL app.tenant_id` 封裝
- [x] **跨租戶隔離測試**(A 看不到 B)
- [x] 使用者可屬於多個租戶並可切換
- [ ] 限制單一使用者可建立的店家數量(目前任何登入者可無限建立)
- [x] 邀請員工加入(invitations、accept_invitation)
- [x] 成員管理:改角色(僅 owner)、移除 / 自己退出(owner 不可被移除)
- [ ] 接受邀請與登入的限流(防止猜 token / 暴力破解)
- [ ] 正式環境部署的資料庫角色說明(連線使用者非超級使用者)

## M4 業務模組
- [x] 服務項目(services)CRUD
- [x] 員工與可提供服務
- [x] 每週營業時間與休假日
- [x] 分頁、篩選

## M4 後續
- [ ] 休假清單分頁(目前上限 500 筆)
- [ ] 稽核日誌(服務、營業時間異動)

## M4.5 預約核心
- [x] 店家時區設定
- [x] 可預約時段計算
- [x] 預約資料表加排除約束(btree_gist)防重複
- [x] 建立 / 取消 / 改期 API
- [x] 公開預約端點(依子網域識別店家)與限流
- [x] 併發預約測試(同時搶同一時段只有一個成功)
- [x] 顧客 Email 驗證(M6 完成:確認後才占位)
- [ ] 預約時段的緩衝時間(服務之間留空檔)
- [ ] 以子網域(`acme.app.com`)識別店家;目前用路徑 `/public/shops/{slug}`
- [ ] 員工離職:有預約紀錄的成員目前無法移除(回 409),需要「停用」機制

## M5 權限
- [x] 角色定義:owner / manager / staff(M4 已先做 `require_manager`、`require_manager_or_self`)
- [ ] 權限檢查 extractor
- [ ] 平台超級管理員

## M6 背景任務
- [x] 任務佇列選型與整合:Postgres outbox + 內建 worker(`FOR UPDATE SKIP LOCKED`),不另引入 Redis
- [x] 任務帶租戶上下文:每列資料自帶 `tenant_id`,worker 用最小權限角色 `tenant_worker`
- [x] 預約確認信 / 前一天提醒信 / 邀請信
- [x] Email 驗證:顧客自助預約改為「確認後才占位」
- [x] 失敗重試(指數退避,最多 5 次)、寄出後清空含連結的內容、舊紀錄 30 天後清理
- [x] 取消預約通知信:店家取消「已確認、還沒開始」的預約 → 通知顧客(附重新預約連結);管理者取消別人負責的 → 也通知那位員工;顧客自己取消 → 通知負責的員工。待確認(Email 未驗證)與已過去的預約不寄
- [ ] 取消通知信的缺口:逾期自動取消與「確認時發現時段已被占走」只在畫面上告知,不寄信;店家沒有「取消原因」欄位,信裡也沒有原因
- [ ] 寄信量限制(同一收件者每小時上限),目前只靠「單一 Email 最多 5 筆未來預約」與 IP 限流
- [ ] 提醒信帶管理連結(目前資料庫只存雜湊,提醒信沒有連結)
- [x] 監控:outbox 堆積、failed 數量(M8 的 `/metrics`)

## M7 計費與限額
- [x] 訂閱方案資料表:`plans`(free / pro / business),店家 `plan_id` 預設 free;店家自己沒有任何寫入權限
- [x] 租戶限額:成員數(含待接受邀請)、啟用中的服務數、每月預約數(依店家當地月份)
- [x] 併發安全:`pg_advisory_xact_lock`(每家店 × 每種資源),建立邀請 / 接受邀請 / 建服務 / 重新啟用服務 / 占位預約 / 確認 / 跨月改期都在鎖內檢查
- [x] 方案與用量查詢 API(`GET /t/{slug}/plan`,manager 以上)、公開方案列表(`GET /plans`)
- [x] **Stripe 整合**:結帳、客戶入口、webhook(簽章、去重、亂序)、past_due 寬限、期末取消、稽核。**僅對 stripe-mock 與本機簽章驗證,尚未用真實 Stripe 帳號(test mode)驗證**
- [ ] Stripe:用真實 test mode 帳號完整跑一次(含 Stripe CLI 轉發 webhook、客戶入口的方案切換設定)
- [x] 付款失敗通知信:`invoice.payment_failed` → 寄給該店所有店主(金額、第幾次、下次重試日期或「最後一次」、更新付款方式的連結),寫稽核 `billing.payment_failed`;事件去重;不改方案。**只對本機簽章事件驗證,未用真實 Stripe 驗證**(webhook 要多訂閱 `invoice.payment_failed`)
- [ ] Stripe:降級後超出新方案限額的既有資料不會被處理(只擋新增);付款成功收據 / 付款恢復通知;試用期、年繳、優惠碼
- [ ] 付款失敗信件的已知限制:只寄給店主(沒有「帳務聯絡人」概念);Stripe 本身的付款失敗信(Dashboard 可開)與我們的並存;晚到的舊失敗事件在已恢復後仍會寄信(發票事件沒有亂序保護)
- [ ] API 速率限額(目前只有公開端點的固定限流,沒有依方案區分)
- [ ] 儲存量限額(尚無檔案上傳功能)
- [ ] 降級時超出新上限的既有資料(目前不強制清理,只擋新增;需要決定策略)
- [ ] 每位使用者可建立的店家數量上限
- [ ] 營運人員後台(平台超級管理員)用來改方案

## M8 稽核與可觀測性
- [x] 稽核日誌:與變更同一交易寫入;資料庫只給新增與讀取(沒有 UPDATE / DELETE);detail 不含 Email、電話、token、休假原因
- [x] 稽核查詢 API:`GET /t/{slug}/audit-logs`(manager 以上),可依動作前綴 / 實體 / 操作者 / 時間篩選,游標翻頁
- [x] 結構化日誌:`LOG_FORMAT=json`,每筆請求日誌帶請求 ID 與路由樣板
- [x] 請求 ID:沿用合法的 `X-Request-Id`,否則產生;回應一律帶回
- [x] metrics:`GET /metrics`(Prometheus,需 `METRICS_TOKEN`,未設定則端點不存在)——請求數、延遲、寄信與 worker 計數、outbox 堆積
- [x] 日誌與指標只記路由樣板,不記實際網址(網址裡有顧客的預約管理 token)
- [ ] 稽核日誌保留期限與歸檔(目前永久保留,量大時需分區或歸檔)
- [ ] 稽核日誌匯出(CSV)
- [ ] 沒有被記錄的事件:登入 / 登出 / 註冊、授權失敗(403)、讀取行為
- [ ] 查看顧客個資(顧客清單 / 詳情)沒有稽核紀錄:誰、何時看了哪位顧客
- [ ] 告警規則範例(outbox 積壓、5xx 比例、延遲 P99)
- [ ] 分散式追蹤(OpenTelemetry)
- [ ] `slot_taken` 這條稽核路徑(兩個確認「真正同時」發生)尚無確定性測試

## M9 部署
- [x] Dockerfile:多階段、依賴層快取、非 root(uid 10001)、內建 `HEALTHCHECK`(`tenant-saas healthcheck`)
- [x] 子命令 `serve` / `migrate` / `healthcheck`;migration 用獨立的 migrator 帳號
- [x] 受限的資料庫帳號:`tenant_runtime`(NOINHERIT,碰不到租戶資料表);啟動時檢查,`APP_ENV=production` 帳號過大就拒絕啟動
- [x] CI(`.github/workflows/ci.yml`):fmt、clippy、測試;**受限帳號**的獨立工作(全新叢集 → 佈建 → migrate → 撤銷成員資格 → 跑完整流程);Docker 建置
- [x] 部署文件 `docs/deployment.md`(角色、步驟、環境變數、反向代理、監控、備份、上線前檢查表)
- [x] 註冊 / 登入限流(防暴力破解)
- [x] 帳號鎖定:同一帳號連續登入失敗 5 次鎖 15 分鐘(補上依 IP 限流擋不住的分散式猜密碼);鎖定中連正確密碼都不收、對外與「密碼錯誤」「帳號不存在」同一個 401;鎖定時寄信通知;重設密碼解鎖;日誌與指標(`account_locked_total`、`login_rejected_locked_total`)
- [ ] 帳號鎖定沒寫進 `audit_logs`(稽核表屬於店家,帳號層級事件沒有 tenant_id);目前只有日誌 + 指標 + 通知信。另外,攻擊者可故意輸錯把別人鎖 15 分鐘(無法登入但不影響忘記密碼),尚無 CAPTCHA
- [x] production 沒設 `SMTP_URL` 就拒絕啟動(Log 模式會把一次性連結印進日誌)
- [x] CI 已在 GitHub 上實際執行(每次 push 觸發,5 個工作:test / restricted-role / web / 2 個 docker)。曾抓到一次間歇性失敗:`CREATE ROLE tenant_runtime` 在平行測試間的競態(見 decisions)
- [ ] 正式環境的 TLS 連線資料庫(`sslmode=require`)尚未實測
- [ ] 映像漏洞掃描、SBOM、簽章
- [ ] docker-compose 的正式環境範例(目前只有文件步驟,沒有可直接執行的 compose)
- [ ] 其他尚未做的上線前項目見 `docs/deployment.md` 檢查表(Stripe、忘記密碼、帳號鎖定、歸檔、前端…)

## 前端(顧客預約頁,`web/`)
- [x] React + TypeScript + Vite;服務列表、選人員 / 日期 / 時間、填資料、Email 確認提示、預約管理(確認 / 改期 / 取消)
- [x] 時間依店家時區顯示;日期週列(沒空檔劃掉)、時段分上午 / 下午 / 晚上、空狀態引導「前往最近可預約日」
- [x] 測試 79 項(Vitest + Testing Library + MSW);突變驗證;前端 CI 工作;nginx 映像並實際驗證標頭與代理
- [x] 深色模式、色彩對比度(WCAG AA)、手機版
- [x] **店家後台**:登入 / 註冊、建店、預約(列表、代客預約、取消、完成 / 未到、備註)、服務、團隊(邀請、角色、移除)、營業時間 / 可提供的服務 / 休假、稽核日誌、方案與用量;測試 185 項(含 9+ 個突變驗證)
- [x] 後台:忘記密碼 / 重設密碼(不洩漏帳號是否存在、每小時 3 封、單次 token、重設後舊登入失效)
- [x] 後台:顧客清單與歷史(搜尋、未到次數、預約紀錄;只列確認過的顧客,僅管理者以上)
- [x] 後台:預約的改期(員工替顧客改期,寄通知信給顧客;員工只能動自己的)
- [x] 後台:週日曆檢視(重疊的並排、已取消不畫、點開看詳情)
- [x] 後台:月檢視(點日期進當天列表、點預約看詳情)
- [x] 後台:週日曆拖曳改期(拖到別天 / 別的時間,吸附 15 分鐘,放開先跳確認再送出;能不能改由後端判斷)。**限制:只支援滑鼠**(HTML5 拖放不支援觸控),鍵盤與手機請用詳情裡的「改期」;月檢視與列表不能拖
- [x] 後台:用 HttpOnly cookie 取代 localStorage 存 JWT(`__Host-session`、SameSite=Strict、Origin 白名單防 CSRF、`POST /auth/logout`)。**尚未用真實 HTTPS 環境驗證 `__Host-`/`Secure` cookie**(本機只有 http 開發環境);部署時請實測登入
- [ ] JWT 無法主動撤銷(登出只清 cookie,偷到 token 的人到期前仍可用 Bearer);要做需要 refresh token 或伺服器端 session / token 黑名單
- [x] 後台:變更店家名稱 / 時區(僅店主;網址代稱不可改)
- [ ] 後台:刪除預約、匯出;稽核日誌匯出
- [ ] 後台的端對端(瀏覽器)自動化測試;目前是元件整合測試 + 手動操作
- [x] 改期時自己原本的時段不再顯示為忙碌(改用「依預約」的可預約時段端點)
- [x] 順便修掉的 bug:顧客改期後不會再收到新時間的提醒信(`reminder_queued_at` 沒重置、去重鍵沒變)
- [x] 沒收到確認信可「重新寄送」(預約編號 + Email 確認身分,換新 token、舊連結失效、最多 3 次、間隔 60 秒、不洩漏預約是否存在)
- [x] 加到行事曆(.ics,改期後重新下載會更新同一個事件)
- [ ] 地圖 / 店家資訊 / 營業時間(後端沒有店家簡介、地址欄位)
- [ ] 多語系(目前只有繁體中文)、幣別(後端沒有欄位,暫以新台幣顯示)
- [ ] 瀏覽器端的端對端測試(Playwright);目前是元件整合測試 + 手動操作
- [ ] 頁面載入後「今天」不會在跨午夜時自動更新
