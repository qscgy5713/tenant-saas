# 顧客預約頁(前端)

React 19 + TypeScript + Vite。給店家的顧客用:選服務 → 選日期時間 → 填資料 → 收信確認 → 管理預約(確認 / 改期 / 取消)。
**只有顧客端**;店家後台(登入、服務與營業時間設定、預約列表…)尚未開發。

## 開發
```bash
# 1. 後端(另一個終端機;SMTP 用本機的 Mailpit,信件看 http://127.0.0.1:8025)
docker compose up -d postgres mailpit
SMTP_URL=smtp://127.0.0.1:1025 PUBLIC_BASE_URL=http://127.0.0.1:5173 cargo run
python3 scripts/seed_demo.py           # 建立示範店家 demo-salon

# 2. 前端
cd web && npm install && npm run dev   # http://127.0.0.1:5173/s/demo-salon
```
`PUBLIC_BASE_URL` 要指向前端:信中的連結是 `${PUBLIC_BASE_URL}/bookings/{token}`。
開發時 Vite 把 `/api/*` 代理到後端(去掉 `/api` 前綴),瀏覽器看到的是同源,不需要 CORS。

## 指令
| 指令 | 說明 |
|---|---|
| `npm run dev` | 開發伺服器 |
| `npm test` | 單元與整合測試(Vitest + Testing Library + MSW) |
| `npm run typecheck` / `npm run lint` | 型別檢查 / oxlint |
| `npm run format` / `format:check` | Prettier |
| `npm run build` | 正式建置到 `dist/` |

## 路由
| 路徑 | 頁面 |
|---|---|
| `/s/:slug` | 店家的服務列表 |
| `/s/:slug/book/:serviceId` | 選人員(多位時)、日期、時間 → 填資料 → 「請到信箱確認」 |
| `/bookings/:token` | 信中的連結:確認 / 改期 / 取消。**確認要按按鈕才送出**(信件掃描器會自動點開連結) |

## 設計重點
- **時間一律以店家時區顯示**,不是瀏覽器的時區;日期用 `YYYY-MM-DD` 字串表示店家當地的日曆日,避免 `Date` 在不同時區解讀出不同的日子(`src/lib/time.ts`)。
- 日期是橫向週列(沒空檔的日期劃掉並標示),時段依上午 / 下午 / 晚上分組;載入中的日期是中性狀態,不會先被當成「沒有空檔」。
- 查詢可預約時段每次最多 14 天(後端限制),一頁顯示 7 天,每兩頁共用一次查詢。
- 預約 / 確認 / 取消這類有副作用的請求**絕不自動重試**;查詢的 4xx 也不重試。
- 5xx 的內容不顯示給使用者,只顯示固定訊息與錯誤代碼(`X-Request-Id`)。
- 深色模式跟隨系統;色彩對比度已檢查符合 WCAG AA。
- 價格:後端只有 `price_cents`,沒有幣別,目前一律以新台幣顯示。
