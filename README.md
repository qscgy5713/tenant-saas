# Tenant SaaS

以 Rust(axum + sqlx + PostgreSQL)實作的多租戶 SaaS 後端。

## 文件
- [plan.md](docs/plan.md):專案計畫與架構
- [todo.md](docs/todo.md):待辦清單
- [worklog.md](docs/worklog.md):工作日誌
- [decisions.md](docs/decisions.md):設計決策紀錄

## 狀態
M1–M5 完成(骨架、認證、多租戶隔離、服務與排程設定、邀請與成員管理),下一步 M4.5 預約核心。

## 快速開始
```
cp .env.example .env
docker compose up -d postgres redis
cargo run
curl 127.0.0.1:3001/health
```

## 目前的 API
| 方法 | 路徑 | 說明 |
|---|---|---|
| GET | `/health` | 健康檢查(含資料庫) |
| POST | `/auth/register`、`/auth/login` | 註冊、登入,回傳 JWT |
| GET | `/auth/me` | 目前使用者 |
| POST / GET | `/tenants` | 建立店家 / 列出我所屬的店家 |
| GET | `/t/{slug}/me` | 我在該店的角色 |
| GET | `/t/{slug}/members` | 該店成員 |
| GET / POST | `/t/{slug}/services` | 服務項目清單(`?active=&limit=&offset=`)/ 新增(manager) |
| GET / PATCH / DELETE | `/t/{slug}/services/{id}` | 查詢 / 局部更新 / 刪除(manager) |
| GET / PUT | `/t/{slug}/members/{user_id}/services` | 員工可提供的服務(PUT 為 manager) |
| GET / PUT | `/t/{slug}/members/{user_id}/working-hours` | 每週營業時間(PUT 為 manager 或本人),weekday 0 = 週日 |
| GET / POST | `/t/{slug}/members/{user_id}/time-off` | 休假(POST 為 manager 或本人) |
| DELETE | `/t/{slug}/time-off/{id}` | 刪除休假(manager 或本人) |
| GET / POST | `/t/{slug}/invitations` | 待處理邀請 / 建立邀請(回傳一次性 token) |
| DELETE | `/t/{slug}/invitations/{id}` | 撤銷邀請 |
| POST | `/invitations/accept` | 被邀請者以 token 加入店家(需登入且 Email 相符) |
| PATCH | `/t/{slug}/members/{user_id}` | 改角色(僅 owner) |
| DELETE | `/t/{slug}/members/{user_id}` | 移除成員 / 自己退出(owner 不可被移除) |

## 測試
```
docker compose up -d postgres
cargo test
```
