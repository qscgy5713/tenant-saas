# 前端(顧客預約頁 + 店家後台)

React 19 + TypeScript + Vite。給店家的顧客用:選服務 → 選日期時間 → 填資料 → 收信確認 → 管理預約(確認 / 改期 / 取消)。
店家後台在 `/admin`,是獨立的程式碼 chunk(`React.lazy`),顧客的預約頁不會下載它。

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
| `/admin/login`、`/admin/register` | 店家後台的登入 / 註冊 |
| `/admin` | 我的店家(選擇或建立) |
| `/admin/:slug` | 總覽(今天的行程、待確認、本月用量) |
| `/admin/:slug/bookings` | 預約:依日期 / 待確認 / 近 30 天、代客預約、取消、完成 / 未到、備註 |
| `/admin/:slug/services` | 服務(新增、編輯、啟用 / 停用、刪除) |
| `/admin/:slug/team`、`team/:userId` | 團隊(邀請、角色、移除);成員的營業時間、可提供的服務、休假 |
| `/admin/:slug/audit`、`plan` | 稽核日誌、方案與用量(僅擁有者 / 管理者) |
| `/invitations/accept#token=…` | 邀請信的連結:登入或註冊後接受邀請 |

## 設計重點
- **時間一律以店家時區顯示**,不是瀏覽器的時區;日期用 `YYYY-MM-DD` 字串表示店家當地的日曆日,避免 `Date` 在不同時區解讀出不同的日子(`src/lib/time.ts`)。
- 日期是橫向週列(沒空檔的日期劃掉並標示),時段依上午 / 下午 / 晚上分組;載入中的日期是中性狀態,不會先被當成「沒有空檔」。
- 查詢可預約時段每次最多 14 天(後端限制),一頁顯示 7 天,每兩頁共用一次查詢。
- 預約 / 確認 / 取消這類有副作用的請求**絕不自動重試**;查詢的 4xx 也不重試。
- 5xx 的內容不顯示給使用者,只顯示固定訊息與錯誤代碼(`X-Request-Id`)。
- **只有淺色主題**,刻意不跟隨系統的深色模式:這是給一般顧客的預約頁,預設就該明亮友善。色彩對比度已逐組計算,文字 ≥ 4.5:1,可互動元件的邊框 ≥ 3:1(WCAG AA / 1.4.11)。
- 沒有照片時,店家標題區用漸層 + 店名縮寫頭像;服務人員同樣用縮寫頭像(顏色由名字決定,同名永遠同色)。圖示是內建的 SVG,不引入圖示套件。
- 頭像的顏色與尺寸都用 class,不用行內 `style`(正式環境的 CSP 是 `style-src 'self'`)。
- 價格:後端只有 `price_cents`,沒有幣別,目前一律以新台幣顯示。

## 後台的設計重點
- **登入狀態**靠後端發的 HttpOnly cookie,前端的 JavaScript 讀不到、也不存任何 token;前端只記「登入的是誰」,重新整理時用 `GET /auth/me` 向後端確認(確認完成前顯示載入,不會閃一下登入頁)。另一個分頁登入 / 登出會透過一個無機密的 localStorage 訊號同步。API 必須與網頁同源(開發時由 Vite 代理 `/api`,正式環境由 nginx)。
- **登出、登入過期、後端回 401** 都會清掉所有快取,下一位在同一台電腦登入的人看不到上一位的資料。
- **登入後只會回到站內的 `/admin`、`/invitations` 路徑**(防開放式重新導向)。
- **邀請連結的 token 放在 `#` 之後**(`/invitations/accept#token=…`):瀏覽器不會把 `#` 後面的內容送到任何伺服器,不會出現在存取紀錄或 Referer。
- **權限只決定「要不要顯示」**:員工看不到稽核 / 方案的入口,直接輸入網址會看到「沒有權限」,而且根本不會打那兩支 API;真正的檢查一律在後端。
- **CSP**:正式環境是 `style-src 'self'; script-src 'self'`,**行內 `style` 屬性會被擋掉**(開發環境的 Vite 沒有這個限制,所以「開發時正常、上線後壞掉」)。`src/csp.test.ts` 會在測試時掃描並擋下行內 style、行內 script、`eval`、`dangerouslySetInnerHTML`。
- 時間輸入(營業時間、休假)一律以店家時區解讀;`lib/time.ts` 的 `zonedToUtc` 處理夏令時間(跳時取跳完後的第一個瞬間、回撥取較早的,與後端規則一致)。
