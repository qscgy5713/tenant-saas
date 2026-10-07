-- 備份專用帳號。以資料庫管理員(superuser)執行一次:
--
--   psql "postgres://admin@host/postgres" \
--        -v backup_password="'換成強密碼'" \
--        -f deploy/provision_backup.sql
--
-- 為什麼不能用 migrator 備份:customers、services 等資料表是 FORCE ROW LEVEL SECURITY,
-- 連擁有者都受 RLS 約束;pg_dump 會關掉 row_security,遇到這種表直接失敗
-- (若改用 --enable-row-security,則會「成功」但那些表是空的 —— 更危險)。
--
-- 所以備份帳號要:
--   * BYPASSRLS:看得到所有租戶的資料(備份本來就要整份)
--   * pg_read_all_data:所有表的讀取權限,**沒有任何寫入權限**
--   * 不是超級使用者、不擁有任何物件
-- BYPASSRLS 只有超級使用者能授予。託管資料庫若不允許,改用服務商提供的管理員帳號備份,
-- 並用「還原後比對筆數」確認 customers 等表不是空的(見 docs/deployment.md)。
CREATE ROLE tenant_backup LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE BYPASSRLS PASSWORD :backup_password;
GRANT pg_read_all_data TO tenant_backup;
