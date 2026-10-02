# Worklog

每次工作結束記錄:做了什麼、遇到什麼問題、下一步。最新的放最上面。

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
