# Worklog

每次工作結束記錄:做了什麼、遇到什麼問題、下一步。最新的放最上面。

## 2026-10-02(M9 部署)
- Dockerfile(172MB、非 root、健康檢查)、`.dockerignore`、CI 工作流程、`deploy/provision.sql`、`docs/deployment.md`
- 子命令:`serve`(預設)、`migrate`、`healthcheck`;`migrate` 用 `MIGRATION_DATABASE_URL`
- `dbrole.rs`:啟動時檢查連線帳號(超級使用者 / BYPASSRLS / 擁有資料表 / 可直接讀租戶資料表);`APP_ENV=production` 過大就拒絕啟動
- migration `0010_runtime_role.sql`(`tenant_runtime`:NOINHERIT,只直接讀寫 users / 讀 plans)、`0011_bookings_no_force.sql`
- `tests/restricted_role.rs`(預設 `#[ignore]`):用受限帳號跑完整流程,並驗證「直接查租戶資料表必須 permission denied」
- 測試共 101 項(+1 項受限帳號測試需獨立叢集)
- **用真實的非超級使用者環境驗證,抓到三個一般測試永遠看不到的問題**:
  1. `booking_tenant_by_token()`(SECURITY DEFINER)讀 FORCE RLS 的 bookings,只因為 migrator 碰巧是 `tenant_worker` 成員才能運作;換一個沒有這個成員資格的 migrator,**所有顧客的預約連結會悄悄變成 404**。已重現並以 migration 0011 修正,CI 會撤銷成員資格後再驗證。**我一開始的假設是「整個流程會壞」,實際測試通過了**——查下去才發現是意外的權限在撐著,而不是我原本想的原因
  2. `sqlx::migrate!` 在編譯時嵌入 migration,Cargo 不追蹤 `migrations/` 資料夾:單獨新增 migration 檔,程式會回報「migration 完成」卻沒套用。加 `build.rs` 修正。先前每次新增 migration 都剛好同時改了原始碼,所以沒碰到
  3. production 沒設 `SMTP_URL` 時走 Log 模式,會把含一次性連結的信件內容印進日誌(演練容器時在日誌裡看到)。production 現在拒絕啟動
- 另外補上註冊 / 登入限流(M2 起一直標為待辦的真實缺口)
- 容器演練:超級使用者 / migrator 當連線帳號 → 拒絕啟動(訊息清楚);受限帳號 → healthy、非 root、真實 SMTP 寄出、日誌與指標無 token、`docker stop` 0.2 秒乾淨退出(exit 0)
- 未驗證:CI 在 GitHub 上實際執行(只驗證 YAML 語法與本機重現每一步);TLS 連資料庫;跨實例限流
- 下一步:所有里程碑完成。可選:Stripe、忘記密碼、前端、帳號鎖定;或先收尾 `todo.md` 的待辦

## 2026-10-02(M8 稽核日誌與可觀測性)
- migration `0009_audit.sql`:`audit_logs`(只能新增)、`create_tenant()` 與 `accept_invitation()` 改為在函式內寫稽核(兩者執行時沒有租戶上下文)
- `audit.rs` + 各 handler:服務、成員、邀請、營業時間、休假、預約(員工 / 顧客 / 系統三種操作者)全部記錄;系統取消會帶原因(`monthly_limit`、`no_longer_bookable`、`slot_taken`、`confirmation_expired`)
- `observability.rs`:請求 ID、只記路由樣板的日誌 span、Prometheus 指標、受保護的 `/metrics`(含 outbox 堆積);`LOG_FORMAT=json`
- 測試共 99 項通過(原 81 + 稽核 9 + 可觀測性 6 + 單元 3);三個突變驗證(換回預設日誌 → 洩漏 token、給稽核表 UPDATE 權限、略過稽核寫入)都被測試抓到;並實際啟動程式驗證 JSON 日誌、請求 ID、`/metrics`
- **發現並修掉的真實問題**:
  1. 預設的請求日誌會把完整網址寫進去,而顧客預約管理 token 就在網址路徑裡(`/public/bookings/{token}`),任何能看日誌的人都能取消 / 改期別人的預約。改為只記路由樣板,並用測試鎖住
  2. **M6 的遺留 bug**:員工不能取消「待確認」的預約。M6 當時用字串替換修改狀態檢查,但 `cargo fmt` 已把那行拆成多行,替換靜默沒生效,而且測試沒涵蓋。現在修好並補測試
  3. 我寫的一個測試設了全域環境變數 `DATABASE_URL`,害同程式內並行的 `sqlx::test` 全部在初始化時失敗。改成純函式驗證,不碰環境變數
- **反覆出現的問題**:`cargo fmt` 重排後,Python 字串替換靜默沒套用(M6、M8 各發生過)。之後替換一律 `assert old in s`,改動較大時直接用精確編輯;並逐項 grep 核對 `main.rs` 這類「測試碰不到」的接線
- 決定:稽核 detail 不放任何個資(稽核表只增不減,寫進去就收不回);休假原因、備註內容、Email 都不記,只記「有異動」
- 已知限制:稽核永久保留;未記錄登入 / 授權失敗;`slot_taken` 路徑只在真正同時確認時才觸發,沒有確定性測試
- 下一步:M9 Docker 化、CI(fmt / clippy / test)與部署文件

## 2026-10-02(M7 方案與限額,不含 Stripe)
- migration `0008_plans.sql`:`plans` 表與三個方案、`tenants.plan_id`、`accept_invitation()` 加入人數檢查;`tenant_app` / `tenant_worker` 對 plans 只讀
- `plan.rs`:成員 / 服務 / 每月預約三種限額;每家店每種資源一把 advisory lock,檢查與寫入在同一個臨界區;月份依店家當地時區計算(單元測試含跨年、夏令時間)
- 限額 402,顧客端一律轉成 409「額滿」,不暴露方案名稱與上限
- 測試共 81 項通過(新增方案 14 項 + 單元 2 項)
- **測試品質的教訓**:拿掉鎖做突變驗證時,預約的「併發賽跑」測試 6 次有 5 次仍通過(只是碰運氣才抓得到超賣)。改成確定性測試:測試先持有同一把鎖,驗證請求必定被擋住;並再用突變證明它抓得到(拿掉鎖、把 SQL 鎖 key 改錯、拿掉改期檢查,各自都會讓測試失敗)。賽跑測試保留當作輔助
- review 修正:①改期到另一個月份可繞過每月上限;②公開端點額滿訊息洩漏店家方案(我先前的測試還照著錯的行為斷言 402);③重新啟用已停用的服務也要算入上限,否則「停用再啟用」可繞過
- 決定:Stripe 留到最後,現階段方案只能由營運人員改資料庫(店家無權限,有測試)
- 已知限制:降級後超出上限的既有資料不強制清理,只擋新增;鎖 key 在 Rust 與 SQL 各寫一份(有測試守著,但改動時要同步)
- 下一步:M8 稽核日誌與可觀測性

## 2026-10-02(M6 背景任務與 Email 驗證)
- migration `0007_jobs.sql`:`booking_status` 加 `pending`、bookings 加 `confirmed_at` / `reminder_queued_at`、`email_outbox`、角色 `tenant_worker` 與其明確政策
- **確認後才占位**:顧客自助預約只建立 pending(不在排除約束條件內,不占時段),寄確認信;按確認時才占位,由排除約束把關。回應不含 token,token 只在信裡。補上上一步「Email 未驗證可被大量佔位」的缺口
- 寄信走 outbox(與業務資料同一交易寫入),worker 逐封 `FOR UPDATE SKIP LOCKED` 取信,失敗指數退避(30s、1m、2m、4m),第 5 次失敗標 failed
- 寄出或最終失敗後清空 body(信裡有一次性連結);已完成的紀錄 30 天後清理
- worker 三件事:過期 pending 取消、排提醒信(24 小時內開始且確認時距離開始超過 24 小時)、寄信;各階段獨立,一個出錯不影響其他
- Mailer 三種後端:Log(開發)、SMTP(lettre + rustls)、Memory(測試,可模擬失敗)
- 驗證:測試共 65 項通過;並用 Mailpit 實際走完 SMTP 流程(收到驗證信、按確認、收到確認信)
- review 修正:確認時重新驗證時段(申請後員工可能新增休假);Log 模式要能看到連結才走得完開發流程;退避時間改用資料庫時鐘計算
- 踩坑:既有測試依賴「公開端點直接建立已確認預約」,改成員工端點建立;字串替換因 `cargo fmt` 已重排而沒對上,導致測試其實沒改到——差點誤判成約束失效,查回應內容才發現
- 已知限制:pending 若指定「任一員工」,確認時衝突就失敗而不重新分配;提醒信沒有管理連結;可對他人 Email 觸發驗證信(每店每 Email 最多 5 筆未過期,24 小時後才釋出)
- 下一步:M7 訂閱方案與限額(員工數、每月預約數)

## 2026-10-02(M4.5 預約核心)
- migration `0006_bookings.sql`:customers、bookings(排除約束防重複)、`tenant_id_by_slug()`、`booking_tenant_by_token()`
- `availability.rs`:純運算的時段計算,9 項單元測試(含紐約夏令時間跳時 / 回撥);營業時間以店家本地時間儲存,依 IANA 時區換算
- `booking.rs`:載入排程、驗證、建立(不指定員工時逐一嘗試,用 savepoint 吃掉排除約束違規)、改期(排除自己)
- 公開端點 `/public/shops/{slug}/...`(店家、服務、員工、可預約時段、預約)與 `/public/bookings/{token}`(查看 / 取消 / 改期);員工端 `/t/{slug}/bookings`(列表、代客預約、改狀態)
- 防濫用:公開端點限流(預設每來源每分鐘 60 次)、單一 Email 最多 5 筆未來預約、最多預約 90 天內、既有顧客資料不被覆寫
- 測試:12 項預約整合測試 + 9 項時段單元測試 + 2 項限流單元測試;全部共 51 項通過
- 破壞性驗證:拿掉排除約束後併發測試失敗(觀察到 2 個請求同時成功),證明應用層檢查擋不住併發、約束才是真正的防線
- 設計取捨:時段起點固定 15 分鐘一格;夏令時間「不存在的時刻」整段略過、「重複的時刻」取較早者;限流的 X-Forwarded-For 只信任最右一筆且預設關閉
- 已知風險:Email 未驗證,可被大量假 Email 佔位(M6 寄確認信後處理)
- 下一步:M6 背景任務(確認信、提醒信),順便處理 Email 驗證

## 2026-10-02(M5 邀請與成員管理)
- migration `0005_invitations.sql`:invitations(token 只存 SHA-256、同店同 Email 僅一張待處理邀請)、`accept_invitation()` 函式
- 接受邀請時資料庫檢查「登入者 Email = 被邀請 Email」;無效 / 過期 / 已用 / Email 不符 / 他人猜測,一律同一個 404
- API:建立 / 列出 / 撤銷邀請;接受邀請;改角色(僅 owner);移除成員與自己退出(owner 不可被移除)
- 權限規則 `Role::can_manage`:owner 管 manager 與 staff,manager 只管 staff,staff 不管任何人
- 測試 5 項(完整邀請流程、權限與驗證、過期與撤銷、跨店隔離、角色與移除);全部共 28 項通過
- review 修正:改角色 / 移除前先 `FOR UPDATE` 鎖住該列,避免併發下權限判斷用到過時的角色
- 踩坑:sha2 新版輸出沒有 LowerHex,改手動轉十六進位
- 未做:寄邀請信(M6,目前由邀請者複製連結)、接受邀請限流、所有權轉讓
- 下一步:M4.5 預約核心(可預約時段計算、防重複預約、時區)

## 2026-10-02(M4 業務模組)
- migration `0004_scheduling.sql`:staff_services、working_hours(同員工同天時段用排除約束防重疊)、time_off,皆有 RLS
- API:services CRUD(分頁、`active` 篩選、PATCH 局部更新)、員工可提供服務(PUT 取代整組)、每週營業時間(PUT 取代整組)、休假
- 最小權限檢查先做:改服務限 owner/manager;營業時間與休假限 owner/manager 或本人
- 測試 6 項(權限、驗證、分頁、同人跨店隔離、失敗的更新整批還原、時段相鄰與重疊);全部共 23 項通過
- review 修正:寫入後改在同一交易內讀回(原本偷懶重用 handler);休假「原因」只給主管與本人(隱私);休假清單加上限 500
- 下一步:M5 邀請員工(invitations、accept_invitation)與成員管理,再進 M4.5 預約核心

## 2026-10-02(M3 多租戶核心)
- migration `0002_tenancy.sql`(角色、輔助函式、tenants、memberships、RLS、`create_tenant`)、`0003_services.sql`(代表性租戶表)
- `db::begin_scoped`(SET LOCAL ROLE + set_config)、`TenantCtx` extractor、`POST/GET /tenants`、`/t/{slug}/me`、`/t/{slug}/members`
- 隔離測試 11 項(含連線池重用、角色權限、密碼雜湊欄位無授權),加上 M2 共 17 項全過
- 破壞性驗證:移除 services 的 RLS → 5 項失敗,還原後全過
- 設計修正:memberships 政策原本用 OR,在 A 店上下文會看到自己在 B 店的關係,改成 CASE
- 踩坑:sqlx 0.9 禁止動態 SQL 字串(測試改用固定字串)
- 未做 / 已知風險:任何登入者可無限建店;`users` 表仍可被 tenant_app 讀 email/name(已限制欄位);正式環境角色設定要另外文件化
- 下一步:M4 服務項目 / 員工 / 營業時間 CRUD,並做 RBAC(M5)

## 2026-10-02(M2 認證)
- migration `0001_users.sql`(citext、btree_gist、users)
- 拆出 `lib.rs`;新增 `error.rs`、`auth.rs`(argon2 雜湊、JWT HS256、`AuthUser` extractor)、`routes/auth.rs`(register / login / me)
- 帳號不存在時也驗一次假雜湊,縮小登入時間差;密碼錯誤與帳號不存在回應相同
- 整合測試 6 項(sqlx::test,每個測試獨立資料庫)全過;clippy 無問題
- Bug:`citext = text`(sqlx 以 text 送參數)會變成區分大小寫,登入改用 `$1::citext`,由測試抓到
- 決定:JWT 只放 user id,不放租戶,切換店家不必重新登入
- 未做:登入限流、refresh token、Email 驗證
- 下一步:M3 租戶、成員、RLS 與隔離測試(需先確認 schema.md 的 6 項決定)

## 2026-10-02(確定領域)
- 領域確定為預約排程,移除所有充電相關內容
- M1 補做 code review:連線池逾時 3 秒、優雅關閉、docker port 綁 127.0.0.1;Redis 驗證 PONG
- 完成 `docs/schema.md`(資料表、RLS 政策、權限對照、隔離測試清單),尚未寫 migration
- 下一步:等使用者確認 schema 的 6 項決定,再進 M2 認證 / M3 migration

## 2026-10-02(M1 完成)
- 安裝 Rust 1.99.0(rustup `-y`)
- `cargo init`,加入 axum 0.8、tokio、sqlx 0.9(postgres)、tracing、tower-http 等依賴
- 建立 `config`(環境變數)、`routes`(`/health` 會 ping DB)、啟動時自動跑 migration
- `docker-compose.yml`(Postgres 17、Redis 7)、`.env.example`、`.gitignore`
- 驗證:DB 正常回 200、DB 停止回 503;`cargo fmt`、`clippy -D warnings` 無問題
- 問題:`localhost:3000` 被另一個 Docker 容器佔用(IPv6),預設 port 改為 3001
- 未驗證:Redis 容器尚未啟動測試(M1 還沒用到)
- 下一步:M2 認證(users 資料表、註冊、登入)

## 2026-10-02(規劃)
- 評估 Rust 可做的專案,決定做多租戶 SaaS
- 建立專案資料夾與規劃文件(plan.md、todo.md、worklog.md、decisions.md、README.md)
- 下一步:確認領域與環境,開始 M1 專案骨架
