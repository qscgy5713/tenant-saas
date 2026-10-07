# Todo

狀態:`[ ]` 未開始、`[~]` 進行中、`[x]` 完成

## 前置
- [x] 確認領域:預約排程
- [x] 確認本機已安裝 Rust toolchain(1.99.0),Postgres、Redis 走 Docker
- [x] 專案資料夾名稱:`tenant-saas`,與 GitHub 儲存庫同名

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
- [x] 登入限流(防暴力破解):`credential_routes` 掛 `rate_limit_auth`,測試 `login_and_register_are_rate_limited_per_client`;另有帳號層級鎖定
- [x] 評估 users 的暴露面:結論是用**欄位層級權限**而非 SECURITY DEFINER 函式 —— `tenant_app` 只能讀 `users(id, email, name, created_at)`,讀不到 `password_hash`(`tests/it/tenancy.rs` 驗證);`tenant_runtime`(登入用)才有整張表。殘留風險:`tenant_app` 若遭 SQL injection 仍可列出所有使用者的 Email 與名稱(`users` 沒有 RLS)
- [x] 認證 extractor

## M3 多租戶核心
- [x] 確認 `schema.md` 的設計與 6 項決定(照預設)
- [x] tenants、memberships 資料表
- [x] 租戶識別 middleware
- [x] 啟用 RLS 與 policy
- [x] 交易內 `SET LOCAL app.tenant_id` 封裝
- [x] **跨租戶隔離測試**(A 看不到 B)
- [x] 使用者可屬於多個租戶並可切換
- [x] 限制單一使用者可「擁有」的店家數量(`MAX_SHOPS_PER_USER`,預設 5;受邀加入別人的店不算;超過回 402)
- [x] 邀請員工加入(invitations、accept_invitation)
- [x] 成員管理:改角色(僅 owner)、移除 / 自己退出(owner 不可被移除)
- [x] 登入的限流(見上)
- [x] 接受邀請端點(`/invitations/accept`)套用與登入 / 註冊相同的限流(`AUTH_RATE_LIMIT_PER_MIN`)
- [x] 正式環境部署的資料庫角色說明:`docs/deployment.md`「最重要的一件事:資料庫帳號」(migrator / runtime / app / worker),且 `APP_ENV=production` 啟動時檢查帳號夠不夠受限

## M4 業務模組
- [x] 服務項目(services)CRUD
- [x] 員工與可提供服務
- [x] 每週營業時間與休假日
- [x] 分頁、篩選

## M4 後續
- [x] 休假清單分頁:預設只列還沒結束的(進行中與未來,依開始時間),可切換看已結束的(最近結束的在前);`limit` / `offset` + `has_more`(多抓一筆判斷,不另外 count);前端「載入更多」
- [x] 稽核日誌(服務、營業時間異動):`service.created/updated/deleted`、`member.working_hours_replaced`、`member.services_replaced`、`time_off.*`

## M4.5 預約核心
- [x] 店家時區設定
- [x] 可預約時段計算
- [x] 預約資料表加排除約束(btree_gist)防重複
- [x] 建立 / 取消 / 改期 API
- [x] 公開預約端點(依子網域識別店家)與限流
- [x] 併發預約測試(同時搶同一時段只有一個成功)
- [x] 顧客 Email 驗證(M6 完成:確認後才占位)
- [x] 服務的「整理時間」(緩衝):`services.buffer_minutes`(0–120,預設 0),服務結束後員工多久不能接下一筆。顧客看到的開始 / 結束時間不變,公開服務列表也不含這個欄位;員工的下一筆最早 = 結束 + 整理時間。新預約自己的整理時間也不能撞到後面的預約
- [ ] 整理時間的缺口:整理時間是**服務層級**(不分員工、不分前後是什麼服務);沒有「兩筆之間的固定空檔」這種員工層級設定;整理時間可以超出營業時間(員工收拾完才下班),但不會自動延長營業時間
- [ ] 以子網域(`acme.app.com`)識別店家;目前用路徑 `/public/shops/{slug}`(不是小項:要處理 DNS、憑證、cookie 網域與 CSRF 白名單)
- [x] 員工離職:成員「停用」(`memberships.active`)—— 進不了這家店、不出現在可預約名單、不佔方案名額、可重新啟用(會重查名額);有未來已確認預約時不准停用;歷史預約與稽核保留。自己「退出團隊」也走停用
- [x] 停用時改派:`POST /members/{id}/deactivate` 帶 `reassign_to`,把未來已確認的預約整批改派給另一位在職成員(時間不變、顧客收通知信、任何一筆排不進去就整個回滾)
- [x] 停用時自動分配(`auto_reassign`):逐筆挑那個時段有空、提供該服務的其他成員
- [x] 店主移交(對方成為店主、自己降為管理者;要輸入密碼)
- [x] 未驗證帳號 30 天後自動清理
- [x] 帳號安全事件(登入 / 失敗 / 鎖定 / 登出所有裝置 / 改密碼 / 驗證),本人在店家列表頁可看;不記 IP
- [x] 店家資訊(簡介 / 地址 / 電話)顯示在顧客預約頁
- [x] 提醒信連結改期後仍有效(保留最近 10 個)
- [x] 映像 / 依賴漏洞掃描(Trivy,每週一也跑)與 SBOM;映像簽章未做(要先決定 registry)
- [x] `slot_taken` 路徑有確定性測試(trigger 模擬搶先占位)
- [ ] 停用的缺口:改派不能「部分指定」(要嘛全部指定一人、要嘛全部自動);停用中的成員還在「服務人員」篩選下拉(標示已停用,為了看歷史);「檢查有無未來預約」與「新預約進來」之間有極小的競態窗口(可能有 1 筆預約落在剛停用的員工身上,管理者看得到、可取消)

## M5 權限
- [x] 角色定義:owner / manager / staff(M4 已先做 `require_manager`、`require_manager_or_self`)
- [x] 權限檢查:`TenantCtx` extractor 解析店家與角色(停用的成員視為非成員),各端點用 `require_*` / `Role::can_manage` 檢查

## M6 背景任務
- [x] 任務佇列選型與整合:Postgres outbox + 內建 worker(`FOR UPDATE SKIP LOCKED`),不另引入 Redis
- [x] 任務帶租戶上下文:每列資料自帶 `tenant_id`,worker 用最小權限角色 `tenant_worker`
- [x] 預約確認信 / 前一天提醒信 / 邀請信
- [x] Email 驗證:顧客自助預約改為「確認後才占位」
- [x] 失敗重試(指數退避,最多 5 次)、寄出後清空含連結的內容、舊紀錄 30 天後清理
- [x] 取消預約通知信:店家取消「已確認、還沒開始」的預約 → 通知顧客(附重新預約連結);管理者取消別人負責的 → 也通知那位員工;顧客自己取消 → 通知負責的員工。待確認(Email 未驗證)與已過去的預約不寄
- [x] 取消通知信可附原因:店家取消「已確認、還沒開始」的預約時可填(選填,最多 200 字),寫進給顧客的信(管理者取消別人負責的預約時,也寫進給那位員工的信)。**只寫進信**:不存資料庫、不進稽核(稽核只記 `has_reason`)
- [ ] 取消通知信的缺口:逾期自動取消與「確認時發現時段已被占走」只在畫面上告知,不寄信;顧客自己取消沒有原因欄位;原因是店家自由輸入的文字,放在**官方寄出的信**裡 —— 帳號被盜用的人可以藉此寄出任何內容(限 200 字、純文字,但仍是風險)
- [x] 寄信量限制:每位收件者每小時最多 `MAX_MAILS_PER_RECIPIENT_PER_HOUR`(預設 6)封,**跨店家**計算;套用在寄給外部指定收件者的信(公開預約驗證信與重寄、店家代客預約確認信、邀請信);超過回 429(重寄則靜默拒絕)
- [ ] 寄信量限制的缺口:只算「寄給外部指定的人」;顧客取消 / 改期通知、店家取消通知不受限(收件者是已有預約關係的人)。登入鎖定通知與重設密碼信有各自的限制
- [x] 提醒信帶改期 / 取消連結:提醒信寄出時另發一個專用 token(`bookings.reminder_token_hash`,只存雜湊),與確認信的連結並存
- [ ] 提醒信連結的缺口:舊資料(0021 之前已排好提醒的)沒有補發
- [x] 監控:outbox 堆積、failed 數量(M8 的 `/metrics`)

## M7 計費與限額
- [x] 訂閱方案資料表:`plans`(free / pro / business),店家 `plan_id` 預設 free;店家自己沒有任何寫入權限
- [x] 租戶限額:成員數(含待接受邀請)、啟用中的服務數、每月預約數(依店家當地月份)
- [x] 併發安全:`pg_advisory_xact_lock`(每家店 × 每種資源),建立邀請 / 接受邀請 / 建服務 / 重新啟用服務 / 占位預約 / 確認 / 跨月改期都在鎖內檢查
- [x] 方案與用量查詢 API(`GET /t/{slug}/plan`,manager 以上)、公開方案列表(`GET /plans`)
- [x] **Stripe 整合**:結帳、客戶入口、webhook(簽章、去重、亂序)、past_due 寬限、期末取消、稽核。**僅對 stripe-mock 與本機簽章驗證,尚未用真實 Stripe 帳號(test mode)驗證**
- [ ] Stripe:用真實 test mode 帳號完整跑一次(含 Stripe CLI 轉發 webhook、客戶入口的方案切換設定)
- [x] 付款失敗通知信:`invoice.payment_failed` → 寄給該店所有店主(金額、第幾次、下次重試日期或「最後一次」、更新付款方式的連結),寫稽核 `billing.payment_failed`;事件去重;不改方案。**只對本機簽章事件驗證,未用真實 Stripe 驗證**(webhook 要多訂閱 `invoice.payment_failed`)
- [x] 付款恢復通知:`invoice.paid` 付清「先前失敗過的那張發票」→ 通知店主、稽核 `billing.payment_recovered`(webhook 要多訂閱 `invoice.paid`)。**一般續訂的 `invoice.paid` 不寄信**(決定不寄例行收據,Stripe 本身可寄收據)
- [ ] Stripe:試用期、年繳、優惠碼
- [ ] 付款失敗信件的已知限制:只寄給店主(沒有「帳務聯絡人」概念);Stripe 本身的付款失敗信(Dashboard 可開)與我們的並存;(晚到的舊失敗事件已處理:發票已付清就不再寄,見 0026)
- [ ] API 速率限額(目前只有公開端點的固定限流,沒有依方案區分)
- [ ] 儲存量限額(尚無檔案上傳功能)
- [ ] 降級時超出新上限的既有資料(目前不強制清理,只擋新增;需要決定策略)
- [x] 每位使用者可建立的店家數量上限(同上,不依方案區分;若要「付費方案可建更多店」需改成依方案)
- [x] 營運人員維運指令(`tenant-saas admin plans|shops|show|set-plan|suspend|unsuspend|export-audit`),改資料的寫 `operator.*` 稽核;**不做網頁後台**(決定見 decisions)
- [ ] 營運人員網頁後台(營運人員超過幾位、或需要非工程人員操作時再做)

## M8 稽核與可觀測性
- [x] 稽核日誌:與變更同一交易寫入;資料庫只給新增與讀取(沒有 UPDATE / DELETE);detail 不含 Email、電話、token、休假原因
- [x] 稽核查詢 API:`GET /t/{slug}/audit-logs`(manager 以上),可依動作前綴 / 實體 / 操作者 / 時間篩選,游標翻頁
- [x] 結構化日誌:`LOG_FORMAT=json`,每筆請求日誌帶請求 ID 與路由樣板
- [x] 請求 ID:沿用合法的 `X-Request-Id`,否則產生;回應一律帶回
- [x] metrics:`GET /metrics`(Prometheus,需 `METRICS_TOKEN`,未設定則端點不存在)——請求數、延遲、寄信與 worker 計數、outbox 堆積
- [x] 日誌與指標只記路由樣板,不記實際網址(網址裡有顧客的預約管理 token)
- [x] 稽核日誌保留期限:`AUDIT_RETENTION_DAYS`(預設 730,0 = 永久),到期由背景任務刪除並留 `audit.purged` 收據
- [x] 稽核日誌歸檔:`admin export-audit <天數>`(JSON Lines,只讀),清除前定期執行
- [ ] 稽核歸檔的缺口:要自己設排程與異地保存(沒有自動寫到物件儲存);量大時資料表需分區;保留期限是全系統一個值,店家不能各自調整
- [x] 稽核日誌匯出(CSV):`GET /t/{slug}/audit-logs/export.csv`(管理者以上),同列表的篩選條件 + 日期範圍;UTF-8 BOM、CRLF、**試算表公式注入防護**(`=` `+` `-` `@` 開頭加單引號)、UTC 與店家當地時間兩欄;上限 50,000 筆(超過回 400 要求縮小範圍,不截斷);匯出本身留一筆 `audit.exported`;後台稽核頁有「匯出 CSV」
- [ ] 稽核匯出的缺口:一次載入整個檔案到記憶體(50,000 筆約 10 MB,所以才有上限);沒有串流 / 背景產生;只有 CSV
- [x] 帳號層級事件(註冊、登入、失敗、鎖定、登出所有裝置、重設密碼、驗證 Email)記在不帶租戶的 `account_events`(0033),本人可在店家列表的「最近的帳號活動」看
- [ ] 沒有被記錄的事件:授權失敗(403)只有日誌與指標;單一裝置的登出不記(沒有伺服器端 session)
- [x] 查看顧客個資留稽核紀錄:`customer.listed`(筆數、是否搜尋、offset)與 `customer.viewed`(顧客 id、預約紀錄筆數);**不記搜尋字串**(常常就是姓名 / 電話 / Email);失敗或沒權限的不留紀錄
- [ ] 查看顧客個資的缺口:預約列表(含顧客姓名 / Email)、預約詳情、員工自己行程裡的顧客資料沒有稽核(日常操作,每次都記會淹沒真正重要的紀錄)
- [x] 告警規則(`deploy/alerts.yml` 9 條 + `alerts_test.yml` 行為測試,CI 的 `deploy-config` 工作會跑)。門檻是起點,沒有在真實流量下調校過
- [ ] 分散式追蹤(OpenTelemetry)

## M9 部署
- [x] Dockerfile:多階段、依賴層快取、非 root(uid 10001)、內建 `HEALTHCHECK`(`tenant-saas healthcheck`)
- [x] 子命令 `serve` / `migrate` / `healthcheck`;migration 用獨立的 migrator 帳號
- [x] 受限的資料庫帳號:`tenant_runtime`(NOINHERIT,碰不到租戶資料表);啟動時檢查,`APP_ENV=production` 帳號過大就拒絕啟動
- [x] CI(`.github/workflows/ci.yml`):fmt、clippy、測試;**受限帳號**的獨立工作(全新叢集 → 佈建 → migrate → 撤銷成員資格 → 跑完整流程);Docker 建置
- [x] 部署文件 `docs/deployment.md`(角色、步驟、環境變數、反向代理、監控、備份、上線前檢查表)
- [x] 註冊 / 登入限流(防暴力破解)
- [x] 帳號鎖定:同一帳號連續登入失敗 5 次鎖 15 分鐘(補上依 IP 限流擋不住的分散式猜密碼);鎖定中連正確密碼都不收、對外與「密碼錯誤」「帳號不存在」同一個 401;鎖定時寄信通知;重設密碼解鎖;日誌與指標(`account_locked_total`、`login_rejected_locked_total`)
- [ ] 帳號鎖定的缺口:攻擊者可故意輸錯把別人鎖 15 分鐘(無法登入但不影響忘記密碼),尚無 CAPTCHA。(鎖定本身已記在 `account_events`,不進店家的 `audit_logs`)
- [x] production 沒設 `SMTP_URL` 就拒絕啟動(Log 模式會把一次性連結印進日誌)
- [x] CI 已在 GitHub 上實際執行(每次 push 與每週一觸發,8 個工作:test / restricted-role / web / e2e / deploy-config / dependency-scan / 2 個 docker)。曾抓到一次間歇性失敗:`CREATE ROLE tenant_runtime` 在平行測試間的競態(見 decisions)
- [x] 資料庫 TLS 連線實測(`require` 與 `verify-full`,應用程式 / migration / 備份);順便發現並修正:**migrator 帳號備份不了 FORCE RLS 的表**,改用專用的 `tenant_backup`(`deploy/provision_backup.sql`),CI 每次實際備份 + 還原比對
- [x] 更換 Email(確認信寄到新地址,點了才換;通知舊地址;寄到舊地址的連結作廢)
- [ ] 映像簽章(要先決定 registry)
- [x] 正式環境 compose 範例(`deploy/docker-compose.prod.yml`)。**已在本機完整演練(建置、migrate、production 模式啟動、真實瀏覽器流程、Prometheus 規則,見 deployment「Docker Compose 部署範例」),但沒有在真實主機與真實 HTTPS 上驗證**;不含資料庫(請用託管 PostgreSQL)
- [ ] 其他尚未做的上線前項目見 `docs/deployment.md` 檢查表「尚未做」區(營運人員後台、跨實例限流、負載測試)

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
- [x] 後台:用 HttpOnly cookie 取代 localStorage 存 JWT(`__Host-session`、SameSite=Strict、Origin 白名單防 CSRF、`POST /auth/logout`)。**已在 production 堆疊(`http://localhost`,Chromium 視為安全來源)驗證 `__Host-session` + Secure 能正常登入與重新整理;尚未用真實 HTTPS 網域驗證**,部署時請實測登入
- [x] JWT 撤銷:改密碼 / 重設密碼 / 「登出所有裝置」都會讓舊 token 立刻失效(每個請求查資料庫)
- [ ] 單一裝置登出(只清 cookie,偷到 token 的人到期前仍可用 Bearer,除非改密碼或登出所有裝置);沒有「登入中的裝置」清單 —— 要做需要伺服器端 session 表
- [x] 註冊 Email 驗證:沒驗證不能建立店家(其他功能不受影響);重寄每小時 3 封;邀請 / 重設密碼連結也算驗證
- [ ] Email 驗證 / 更換的缺口:30 天內沒驗證(也沒加入店家)的帳號會被清理;更換 Email 不會搬移寄到舊地址、還沒接受的邀請(要請店家重新邀請);被盜用者換掉 Email 後,原主人只能聯絡營運人員取回(沒有自助申訴流程)
- [x] 後台:變更店家名稱 / 時區(僅店主;網址代稱不可改)
- [x] 後台:匯出預約(CSV,管理者以上):日期範圍(與期間重疊的預約,含已取消)、公式注入防護、UTC 與店家當地時間、上限 50,000 筆、匯出留稽核 `booking.exported`(不含個資)
- [x] 刪除顧客個資(匿名化,僅擁有者,不可還原):姓名 / Email / 電話換成無法還原的代號,預約備註清空,沒寄出的信刪除、已寄出的信件紀錄改匿名地址;預約本身保留(統計);還有未來 / 待確認預約時拒絕;留稽核 `customer.anonymized`
- [ ] 刻意**不做**「硬刪除預約」:預約有稽核與統計的關聯,取消已涵蓋「這筆不要了」;要刪的是個資,見上
- [x] 資料保留:顧客個資到期自動匿名化(每店可調 90–3650 天)、稽核日誌到期清除、店主申請刪除店家(30 天寬限、可取消、期滿硬刪除)
- [x] 註冊量告警(`registrations_total`、`RegistrationSpike`:10 分鐘超過 30 個)
- [ ] 註冊濫用:註冊會對任意 Email 寄驗證信(每 IP 每分鐘 10 次),可被拿來騷擾別人的信箱;**沒有 CAPTCHA(決定先不加,見 decisions)**,只有告警。伺服器時鐘差異會讓撤銷 / 改密碼的邊界偏移(要 NTP)
- [x] 使用者帳號刪除(本人,要輸入密碼):匿名化、Email 釋出、所有登入失效;店主要先刪店、有未來預約要先處理
- [x] 備份腳本 `deploy/backup.sh`(原子寫入、備份後驗證、依天數清理、保護最新幾份)+ 測試(CI);已用真實 pg_dump / pg_restore 演練
- [ ] 帳號刪除的缺口:刪除後該 Email 的舊稽核仍指著匿名帳號(id),不含個資;沒有刪除前的資料匯出
- [ ] 備份的缺口:腳本只負責產生與清理 —— **異地複製、加密、備份帳號的建立、定期還原演練、角色備份都要自己處理**;從備份還原會讓已刪除的資料復活(靠日誌的 id 重做,見 deployment)
- [ ] 資料保留的缺口:匿名化前不會通知顧客或店家;一次最多處理 200 位顧客(量大時要多輪);刪除店家前沒有「匯出所有資料」(只有稽核與預約的 CSV);
- [ ] 刪除個資的缺口:不涵蓋應用程式日誌與資料庫備份(請依保留政策處理);沒有顧客自助的「刪除我的資料」入口(目前由店家擁有者代為處理);自動保留期限已做(顧客個資 730 天預設)但不涵蓋備份
- [x] 改期時自己原本的時段不再顯示為忙碌(改用「依預約」的可預約時段端點)
- [x] 順便修掉的 bug:顧客改期後不會再收到新時間的提醒信(`reminder_queued_at` 沒重置、去重鍵沒變)
- [x] 沒收到確認信可「重新寄送」(預約編號 + Email 確認身分,換新 token、舊連結失效、最多 3 次、間隔 60 秒、不洩漏預約是否存在)
- [x] 加到行事曆(.ics,改期後重新下載會更新同一個事件)
- [x] 店家資訊(簡介 / 地址 / 電話,0034)與地圖連結
- [ ] 預約頁沒有列出店家的營業時間(營業時間是每位員工各自的,要先決定「店家營業時間」怎麼定義;顧客目前從可預約時段看得出來)
- [ ] 多語系(目前只有繁體中文)、幣別(後端沒有欄位,暫以新台幣顯示)
- [x] 瀏覽器端的端對端測試(Playwright,`web/e2e/`,7 個檔案 18 個測試,CI 的 `e2e` 工作):註冊 / 登入 cookie / CSRF / 顧客預約全流程(含真實 SMTP 收信)/ 拖曳改期 / 員工停用(含改派)/ 帳號鎖定與重設密碼
- [ ] 端對端測試尚未涵蓋:Stripe 付款流程;可及性只測了鍵盤登入與對話框焦點(沒有螢幕閱讀器);多瀏覽器只有 Chromium + WebKit 的關鍵流程(Firefox 在作者的機器上無法啟動,未驗證)
- [x] 頁面開著跨過午夜,「今天」會自動更新(`useToday(timezone)`:店家時區午夜的計時器 + 分頁重新可見 / 視窗取得焦點時再檢查);套用在總覽、預約頁、週 / 月日曆、顧客的日期選擇(換日時整個重來)
- [ ] 跨午夜更新沒涵蓋:休假表單的預設日期(每次開啟時重新取,不受影響)、「預約已開始」的判斷(另有 `useNow` 每 30 秒更新)
