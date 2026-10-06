#!/usr/bin/env bash
# backup.sh 的測試(不需要資料庫:用假的 pg_dump / pg_restore)。執行:bash deploy/backup_test.sh
set -uo pipefail
cd "$(dirname "$0")"
fail=0
check() { if [ "$2" = "$3" ]; then echo "  ok   $1"; else echo "  FAIL $1(期望 $3,實際 $2)"; fail=1; fi; }

work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
fake="$work/bin"; mkdir -p "$fake"
printf '#!/bin/sh\necho "fake-dump-content"\n' > "$fake/pg_dump_ok"
printf '#!/bin/sh\nexit 0\n' > "$fake/pg_dump_empty"
printf '#!/bin/sh\ncat > /dev/null\n' > "$fake/pg_restore_ok"
printf '#!/bin/sh\ncat > /dev/null\nexit 1\n' > "$fake/pg_restore_bad"
chmod +x "$fake"/*

run() { # DUMP RESTORE [env...]
  local dump="$1" restore="$2"; shift 2
  env BACKUP_DATABASE_URL=postgres://x BACKUP_DIR="$dir" PG_DUMP="$fake/$dump" PG_RESTORE="$fake/$restore" "$@" bash ./backup.sh > "$work/out" 2>&1
}
age() { touch -t "$(date -v-"$2"d +%Y%m%d%H%M 2>/dev/null || date -d "$2 days ago" +%Y%m%d%H%M)" "$1"; }
count() { ls -1 "$dir"/tenant_saas-*.dump 2>/dev/null | wc -l | tr -d ' '; }

echo "1. 正常備份:產生檔案、沒有殘留 .partial、權限 600"
dir="$work/a"; run pg_dump_ok pg_restore_ok; check "結束碼" $? 0
check "檔案數" "$(count)" 1
check "沒有 .partial" "$(ls "$dir"/*.partial 2>/dev/null | wc -l | tr -d ' ')" 0
perm="$(stat -c %a "$dir"/*.dump 2>/dev/null || stat -f %Lp "$dir"/*.dump)"; check "權限" "$perm" 600

echo "2. 清理:超過天數的刪掉、沒超過的留著(保留最新 3 份以外)"
dir="$work/b"; mkdir -p "$dir"
for n in 1 2 3 4 5; do : > "$dir/tenant_saas-2020010${n}-000000.dump"; done
age "$dir/tenant_saas-20200101-000000.dump" 100; age "$dir/tenant_saas-20200102-000000.dump" 90
age "$dir/tenant_saas-20200103-000000.dump" 40;  age "$dir/tenant_saas-20200104-000000.dump" 10
age "$dir/tenant_saas-20200105-000000.dump" 1
run pg_dump_ok pg_restore_ok BACKUP_RETENTION_DAYS=35 BACKUP_KEEP_MIN=2
# 排序後最新 2 份(新備份 + 0105)受保護;0101、0102、0103 超過 35 天 → 刪;0104 只有 10 天 → 留
check "剩下的份數(新備份、0105、0104)" "$(count)" 3
check "0102 被刪" "$(ls "$dir"/tenant_saas-20200102-* 2>/dev/null | wc -l | tr -d ' ')" 0
check "0101 被刪" "$(ls "$dir"/tenant_saas-20200101-* 2>/dev/null | wc -l | tr -d ' ')" 0
check "0103 被刪" "$(ls "$dir"/tenant_saas-20200103-* 2>/dev/null | wc -l | tr -d ' ')" 0
check "0104 還在" "$(ls "$dir"/tenant_saas-20200104-* 2>/dev/null | wc -l | tr -d ' ')" 1

echo "3. 備份壞掉一陣子:舊檔都過期了,也不能清光(永遠保留最新 N 份)"
dir="$work/c"; mkdir -p "$dir"
for n in 1 2 3 4 5; do f="$dir/tenant_saas-2019010${n}-000000.dump"; : > "$f"; age "$f" 400; done
run pg_dump_empty pg_restore_ok BACKUP_RETENTION_DAYS=35 BACKUP_KEEP_MIN=3; check "備份失敗的結束碼" $? 1
# 備份失敗就不會走到清理
check "備份失敗時不清理" "$(count)" 5
run pg_dump_ok pg_restore_ok BACKUP_RETENTION_DAYS=35 BACKUP_KEEP_MIN=3
check "成功後:最新 3 份(含新的)保留" "$(count)" 3

echo "4. 空的備份、驗證失敗的備份都會被丟棄,不留半份"
dir="$work/d"; run pg_dump_empty pg_restore_ok; check "空備份結束碼" $? 1; check "沒留檔案" "$(ls -A "$dir" | wc -l | tr -d ' ')" 0
dir="$work/e"; run pg_dump_ok pg_restore_bad; check "驗證失敗結束碼" $? 1; check "沒留檔案" "$(ls -A "$dir" | wc -l | tr -d ' ')" 0

echo "5. 參數檢查"
dir="$work/f"
run pg_dump_ok pg_restore_ok BACKUP_RETENTION_DAYS=0; check "天數 0 被拒絕" $? 2
run pg_dump_ok pg_restore_ok BACKUP_RETENTION_DAYS=abc; check "天數非數字被拒絕" $? 2
run pg_dump_ok pg_restore_ok BACKUP_KEEP_MIN=0; check "保留份數 0 被拒絕" $? 2
check "被拒絕時沒產生備份" "$(count)" 0

[ "$fail" = 0 ] && echo "全部通過" || { echo "有失敗"; exit 1; }
