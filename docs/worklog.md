# Worklog

每次工作結束記錄:做了什麼、遇到什麼問題、下一步。最新的放最上面。

## 2026-10-02(功能面:店家設定、.ics、顧客清單、月檢視)
- 店家設定(僅店主改店名 / 時區)、顧客可下載 .ics、顧客清單與預約紀錄、後台月檢視
- 過程中的錯誤:一次 Python 取代用 `index()` 命中了同名的更早位置,蓋掉 public.rs 中間一段 → 立刻用 `git checkout` 還原後重做(其餘改動都已提交,所以沒有損失)
- 突變驗證抓到一個無效測試:月曆日期分組的測試資料帶 `+08:00`,以 UTC 分組也會過 → 改用 `Z` 字串
- 下一步:帳號鎖定、拖曳改期、付款失敗通知信

## 2026-10-02(重寄確認信)
- 補做上一輪沒做的真實瀏覽器驗證:週日曆、員工改期(自己的時段可選)、方案頁(未啟用 Stripe)、登入頁連結都正常
- 重寄確認信:`POST /public/shops/{slug}/bookings/{id}/resend`、migration 0015、前端「沒收到?重新寄送」;突變驗證 6 項
- 下一步:.ics、月檢視、顧客清單

## 2026-10-02(員工改期、忘記密碼、週日曆、Stripe)
- 員工替顧客改期:`POST /t/{slug}/bookings/{id}/reschedule`(員工只能動自己的)、兩個「排除預約自己」的可預約時段端點(顧客與員工)、通知信、稽核;前端 `SlotPicker` 改成接受「時段來源」,後台預約列新增「改期」。
- 順手找到並修掉既有 bug:顧客改期後不會再收到新時間的提醒(先寫失敗測試再修;新增 `reminder_seq`)。
- 忘記密碼 / 重設密碼(後端 + 前端 `/admin/forgot`、`/admin/reset`):不洩漏帳號、限次、單次 token、重設後舊登入失效。
- 週日曆:純函式排版(`lib/calendar.ts`,15 分鐘一格、重疊並排)+ 位置全用預先產生的 CSS class(CSP 不允許行內 style);一週超過一頁資料會抓完。
- Stripe:結帳、客戶入口、webhook;見 decisions.md。**驗證方式**:Rust 內建假 Stripe 伺服器的整合測試(13 項)、`stripe-mock`(Stripe 官方規格驗證請求參數)端到端、對執行中伺服器送本機簽章的 webhook。**沒有用真實 Stripe 帳號驗證**,沒有驗證 Stripe 實際送來的事件內容是否與我解析的欄位完全一致(依文件與 stripe-mock 的格式)。
- 突變驗證:提醒重排(4)、忘記密碼(3)、計費(7)、員工改期/時段來源(3)。
- 測試:後端 + 前端數量見最後一次完整執行。
- 下一步:用真實 Stripe test mode 走完整流程;月檢視。

## 2026-10-02(店家後台)
- 範圍:`/admin` 下的完整後台。先看真實產品(Fresha、Square 的後台頁面多為動態載入,看得到的有限,主要參考預約頁的設計語言)。登入 / 註冊、選店家 / 建店、總覽、預約、服務、團隊與成員設定、稽核、方案與用量、接受邀請
- 結構:`web/src/admin/`,以 `React.lazy` 獨立成 chunk(約 17KB gzip),顧客預約頁不下載;側欄(桌機)/ 底部分頁列(手機);重用顧客端的 `SlotPicker`(代客預約與改期)
- 後端小改:邀請信的連結由 `?token=` 改為 `#token=`,token 不再送到伺服器、不進存取紀錄與 Referer(同步更新後端測試)
- 前端測試 185 項(原 79 → +106),後端 101 項不變;前端 CI 步驟本機等價全過
- **突變驗證**:認證 4 組、權限與頁面行為 9 組;其中抓到並處理 3 件事:
  1. `safeReturnPath` 裡的 `!startsWith('//')` 是**死程式碼**(正規式 `^\/(admin|invitations)` 已保證),突變不被抓是因為它等價、不是測試漏洞 → 刪除多餘條件並改註解,另外用「拿掉開頭錨點」這個真正的開放式重新導向突變驗證,被抓到
  2. 「登出清快取」的測試形同虛設:測試客戶端設了 `gcTime: 0`,元件卸載後快取立刻被回收,所以「快取是空的」無論有沒有清都成立。拿掉後,清快取的兩條路徑(登出鈕、登入狀態消失)才各自被測到
  3. **不穩定的測試**(整套平行跑偶爾失敗 3/12):第一個渲染後台的測試要付出 `React.lazy` 冷啟動編譯成本,超過 `findBy` 的 1 秒。不調大逾時(那是蓋住問題),改在 `beforeAll` 預載;修正後同樣壓力 12 次 0 失敗
- **自我 review 抓到的真實 bug**:`PlanPage` 的用量條用行內 `style={{ width }}`,但正式環境 nginx 是 `style-src 'self'`,**行內樣式會被擋,用量條永遠是 0 寬度**;開發環境(Vite)沒有這個 CSP 所以看起來正常。這與先前頭像元件要避免的是同一件事,我又犯了一次。修正為 class,並新增 `src/csp.test.ts` 掃描行內 style / script、`eval`、`dangerouslySetInnerHTML`,突變(把行內 style 放回去)會被抓到;再用真實 nginx 容器 + 瀏覽器驗證寬度正確
- 用量條 10 / 1000 = 1% 被四捨五入成 0,條看起來是空的像沒有用量 → 有用量至少顯示一格(5%),幾乎滿但沒滿最多 95%(不能誤導成額滿);抽成 `lib/meter.ts` 並測試
- 其他 review 修正:自己退出團隊後原本用整頁重載,改為清掉快取再導覽;成員設定頁「返回」用 `..` 會跳到店家首頁(React Router 依路由層級而非路徑),改成明確路徑
- **要誠實說的**:
  - 驗證嚴格 CSP 時,`form_input` 工具填 React 受控表單沒有生效(欄位被重置),我差點把它當成程式問題;用真實鍵盤輸入(`type`)就正常,並確認登入 API 透過 nginx 是 200。是工具與受控元件的互動,不是程式問題
  - 第一次想重現不穩定測試時,我同時跑 12 份 vitest,把工作程序壓垮(`Failed to start forks worker`),結果無效作廢,改用 3 份同時
  - 沒有在真實手機上試過;手機版只用 390px iframe 看過底部分頁列
  - 沒有瀏覽器端的端對端自動化測試
- 下一步:忘記密碼(後端也沒有)、日曆檢視、員工替顧客改期(後端沒有 API)

## 2026-10-02(前端改版:明亮、好看)
- 使用者反映「不要黑黑的」:根因是樣式跟隨系統深色模式,而使用者的電腦是深色。改成只有淺色主題
- 依真實產品再設計一輪(Fresha、Square):暖白底 + 柔和漸層、白色圓角卡片、單一綠色強調色、店家標題區(漸層 + 縮寫頭像)、步驟指示、有頭像的服務人員選擇、今天標記、膠囊時段、完成頁的信封圖示
- 新增 `Icons.tsx`(內建 SVG)、`Avatar.tsx`(class 版,不用行內 style)
- 對比度重新逐組計算,抓到兩項不合格並修正:可互動元件的邊框 1.64:1(需 ≥ 3)、停用主按鈕 3.6:1
- 實際在瀏覽器檢視:服務列表、選時間(桌機 + 390px 手機)、填資料、完成、管理頁(待確認 → 確認)
- 前端 79 項測試維持通過(改版沒有動到文字與角色);型別、lint、排版、建置通過
- 修正:頁首寬度與內容不對齊(窄版頁面的頁首也改為窄版)
- **修正自己造成的文件損壞**:前一個 commit 更新 `decisions.md` 時,我用標題當插入點整段替換,卻沒把標題補回去,「資料庫帳號」那個決策失去標題、只剩一行殘句,而且已經推上去。這次因為替換有 `assert` 才發現,已補回
- 沒做:真實手機測試;深色模式改成「可選」而非預設(目前完全拿掉)

## 2026-10-02(前端:顧客預約頁)
- 範圍:先做顧客端(後端公開 API 已齊全);店家後台留待下一階段
- **先看真實產品再設計**(Cal.com、Square 的店家預約頁):採用橫向週列 + 沒空檔劃掉、「前往最近可預約日」空狀態、明確標示時區、固定的預約摘要
- `web/`:React 19 + TypeScript + Vite + React Router + TanStack Query;`lib/time.ts` 處理店家時區與日曆日、`lib/slots.ts` 分組、`components/SlotPicker.tsx` 為核心
- 後端小改:`GET /public/bookings/{token}` 補上 `shop_slug`、`service_id`、`staff_id`(改期要用,都不是機密)
- 測試:前端 79 項(含 4 個突變驗證);後端維持 101 項;前端 CI 工作;本機等價執行每個步驟
- **用真實後端 + Mailpit 在瀏覽器走完整流程**:選服務 → 選時間 → 驗證 → 送出 → 收信 → 開信中連結 → 確認 → 改期 → 取消;也用 iframe 看了 390px 手機版
- 實際操作與 review 發現並修掉的問題:
  1. 換到下一個兩週視窗時,`keepPreviousData` 會讓畫面用**上一個視窗的資料**判斷,閃出錯誤的「這一週沒有空檔」→ 拿掉
  2. **載入中整排日期被當成「沒有空檔」**(劃掉、螢幕閱讀器唸「沒有空檔」)→ 載入中改為中性狀態,並用手動控制延遲的測試鎖住
  3. `timeZoneName: 'short'` 在 zh-TW 下台北給 `GMT+8`、紐約卻給 `EST`,格式不一致 → 改 `shortOffset`
  4. 深色模式的危險按鈕白字配淺鮭魚底,對比度 2.29 → 改為深色字 7.98(我先前的對比度檢查漏了這一組)
  5. 管理頁的 `Date.now()` 在 render 中不純;頁面開著不動時,跨過預約開始時間按鈕也該消失 → `useNow`
  6. 測試寫死完整日期字串,Node 與 Chrome 的 ICU 版本不同(有無空白)→ 改成容許空白
- **要誠實說的**:
  - 我曾把 nginx 驗證結果當成真的,但 `docker run` 其實因 8080 埠被佔用而失敗,curl 打到的是你機器上別的服務。看到錯誤訊息後全部作廢,換埠重做
  - 瀏覽器視窗縮放有最小寬度,不能直接模擬手機;改用 iframe(iframe 內的 media query 依自己的寬度判斷)
  - 沒有真的在手機上試過;沒有跑瀏覽器端的端對端自動化測試
- 下一步:店家後台(登入、服務與營業時間設定、預約列表、成員、稽核、方案用量)

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
