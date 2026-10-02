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
- [ ] 確認 `schema.md` 的設計與 6 項決定
- [ ] tenants、memberships 資料表
- [ ] 租戶識別 middleware
- [ ] 啟用 RLS 與 policy
- [ ] 交易內 `SET LOCAL app.tenant_id` 封裝
- [ ] **跨租戶隔離測試**(A 看不到 B)
- [ ] 使用者可屬於多個租戶並可切換

## M4 業務模組
- [ ] 服務項目(services)CRUD
- [ ] 員工與可提供服務
- [ ] 每週營業時間與休假日
- [ ] 分頁、篩選

## M4.5 預約核心
- [ ] 店家時區設定
- [ ] 可預約時段計算
- [ ] 預約資料表加排除約束(btree_gist)防重複
- [ ] 建立 / 取消 / 改期 API
- [ ] 公開預約端點(依子網域識別店家)與限流
- [ ] 併發預約測試(同時搶同一時段只有一個成功)

## M5 權限
- [ ] 角色定義(admin / operator / viewer)
- [ ] 權限檢查 extractor
- [ ] 平台超級管理員

## M6 背景任務
- [ ] 任務佇列選型與整合
- [ ] 任務帶租戶上下文
- [ ] 預約確認信、前一天提醒信

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
