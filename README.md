# Tenant SaaS

以 Rust(axum + sqlx + PostgreSQL)實作的多租戶 SaaS 後端。

## 文件
- [plan.md](docs/plan.md):專案計畫與架構
- [todo.md](docs/todo.md):待辦清單
- [worklog.md](docs/worklog.md):工作日誌
- [decisions.md](docs/decisions.md):設計決策紀錄

## 狀態
M1 專案骨架完成,下一步 M2 認證。

## 快速開始
```
cp .env.example .env
docker compose up -d postgres redis
cargo run
curl 127.0.0.1:3001/health
```
