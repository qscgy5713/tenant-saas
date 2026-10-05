#!/usr/bin/env bash
# 端對端測試前置:乾淨的資料庫 + 清空的信箱。
# 本機:順便啟動 docker compose 的 postgres 與 mailpit。CI:這兩個是 service container。
set -euo pipefail
cd "$(dirname "$0")/.."

DB=tenant_saas_e2e

if [ -z "${CI:-}" ]; then
  (cd .. && docker compose up -d postgres mailpit >/dev/null)
fi

if command -v psql >/dev/null 2>&1; then
  PSQL=(psql "postgres://tenant:tenant@localhost:5432/postgres" -v ON_ERROR_STOP=1 -q)
else
  PSQL=(docker compose -f ../docker-compose.yml exec -T postgres psql -U tenant -d postgres -v ON_ERROR_STOP=1 -q)
fi

for _ in $(seq 1 30); do
  "${PSQL[@]}" -c 'select 1' >/dev/null 2>&1 && break
  sleep 1
done

# 每次從全新的資料庫開始(後端啟動時會自動套用 migration)
"${PSQL[@]}" -c "DROP DATABASE IF EXISTS $DB WITH (FORCE)" -c "CREATE DATABASE $DB"

# 清空信箱,測試才不會讀到上一輪的信
for _ in $(seq 1 30); do
  curl -sf -X DELETE http://127.0.0.1:8025/api/v1/messages >/dev/null && exit 0
  sleep 1
done
echo "Mailpit 沒有回應(http://127.0.0.1:8025)" >&2
exit 1
