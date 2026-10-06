#!/usr/bin/env bash
# 資料庫備份 + 到期清理。用法(例如每天由 cron 執行一次):
#
#   BACKUP_DATABASE_URL='postgres://tenant_migrator:密碼@DB_HOST/tenant_saas?sslmode=require' \
#   BACKUP_DIR=/var/backups/tenant-saas \
#   BACKUP_RETENTION_DAYS=35 \
#   deploy/backup.sh
#
# - 必須用「讀得到所有資料表」的帳號(migrator 或專用的備份帳號)。**不要用 runtime 帳號**:它讀不到租戶資料表。
# - 備份是 pg_dump 的 custom 格式;先寫成暫存檔、確認能被 pg_restore 讀取,才改名成正式檔案(不會留下半份備份)。
# - 清理:刪除超過 BACKUP_RETENTION_DAYS 天的備份,**但永遠保留最新的 BACKUP_KEEP_MIN 份**
#   (備份壞掉一陣子時,不能因為「舊檔都過期了」就把僅存的備份清光)。
# - 這個天數就是「刪除資料的請求真正從系統消失」的最長期限(備份裡的資料不會被匿名化 / 刪除),
#   請依你的隱私政策決定,並寫進隱私權說明。見 docs/deployment.md「備份與還原」。
# - 角色(roles)是整個叢集共用、不在資料庫備份裡:另外用 `pg_dumpall --roles-only` 備份。
set -euo pipefail

: "${BACKUP_DATABASE_URL:?請設定 BACKUP_DATABASE_URL}"
: "${BACKUP_DIR:?請設定 BACKUP_DIR}"
RETENTION_DAYS="${BACKUP_RETENTION_DAYS:-35}"
KEEP_MIN="${BACKUP_KEEP_MIN:-3}"
# 可以換成 "docker exec -i 容器 pg_dump" 之類的指令(用空白分隔)
read -r -a PG_DUMP <<< "${PG_DUMP:-pg_dump}"
read -r -a PG_RESTORE <<< "${PG_RESTORE:-pg_restore}"

case "$RETENTION_DAYS" in ''|*[!0-9]*) echo "BACKUP_RETENTION_DAYS 必須是正整數" >&2; exit 2;; esac
case "$KEEP_MIN" in ''|*[!0-9]*) echo "BACKUP_KEEP_MIN 必須是整數" >&2; exit 2;; esac
if [ "$RETENTION_DAYS" -lt 1 ]; then echo "BACKUP_RETENTION_DAYS 不得少於 1" >&2; exit 2; fi
if [ "$KEEP_MIN" -lt 1 ]; then echo "BACKUP_KEEP_MIN 不得少於 1(否則備份壞掉時會被清光)" >&2; exit 2; fi

mkdir -p "$BACKUP_DIR"
umask 077   # 備份裡有所有顧客的資料,只有擁有者能讀
stamp="$(date -u +%Y%m%d-%H%M%S)"
final="$BACKUP_DIR/tenant_saas-$stamp.dump"
tmp="$final.partial"
trap 'rm -f "$tmp"' EXIT

"${PG_DUMP[@]}" --format=custom --no-owner "$BACKUP_DATABASE_URL" > "$tmp"
if [ ! -s "$tmp" ]; then echo "備份是空的" >&2; exit 1; fi
# 確認備份讀得回來(只看目錄,不還原)
if ! "${PG_RESTORE[@]}" --list < "$tmp" > /dev/null; then
  echo "備份驗證失敗:pg_restore 讀不了這份檔案,已丟棄" >&2
  exit 1
fi
mv "$tmp" "$final"
trap - EXIT
echo "已備份:$final($(wc -c < "$final" | tr -d ' ') bytes)"

# 清理:依檔名(時間戳記,由新到舊)排序,保留最新 KEEP_MIN 份,其餘超過天數的才刪
# (用檔案修改時間判斷「過期」;find -mtime +N = 超過 N 個整天)
i=0
pruned=0
while IFS= read -r f; do
  i=$((i + 1))
  [ "$i" -le "$KEEP_MIN" ] && continue
  if [ -n "$(find "$f" -mtime +"$RETENTION_DAYS" -print 2>/dev/null)" ]; then
    rm -f -- "$f"
    pruned=$((pruned + 1))
  fi
done < <(ls -1 "$BACKUP_DIR"/tenant_saas-*.dump 2>/dev/null | sort -r)
echo "清理:刪除 $pruned 份超過 $RETENTION_DAYS 天的舊備份(永遠保留最新 $KEEP_MIN 份)"
