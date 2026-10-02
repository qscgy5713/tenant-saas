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
- [ ] 取消預約通知信(顧客與員工)
- [ ] 寄信量限制(同一收件者每小時上限),目前只靠「單一 Email 最多 5 筆未來預約」與 IP 限流
- [ ] 提醒信帶管理連結(目前資料庫只存雜湊,提醒信沒有連結)
- [ ] 監控:outbox 堆積、failed 數量

## M7 計費與限額
- [ ] 訂閱方案資料表
- [ ] Stripe 整合
- [ ] 租戶限額檢查(員工數、每月預約數、API 速率)

## M8 稽核與可觀測性
- [ ] 稽核日誌
- [ ] 結構化日誌與 request id
- [ ] metrics

## M9 部署
- [ ] Dockerfile
- [ ] CI(fmt、clippy、test)
- [ ] 部署文件
