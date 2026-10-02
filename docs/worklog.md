# Worklog

每次工作結束記錄:做了什麼、遇到什麼問題、下一步。最新的放最上面。

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
