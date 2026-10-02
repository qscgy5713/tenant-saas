-- 正式環境的一次性佈建。以資料庫管理員(superuser 或有 CREATEDB / CREATEROLE 的帳號)執行:
--
--   psql "postgres://admin@host/postgres" \
--        -v migrator_password="'換成強密碼'" \
--        -f deploy/provision.sql
--
-- 之後的步驟(套用 migration、啟用 runtime 帳號)見 docs/deployment.md。
-- 需要 PostgreSQL 16 以上。

-- 套用 migration 的帳號:擁有資料表與函式,可以建立角色(CREATEROLE),但不是超級使用者
CREATE ROLE tenant_migrator LOGIN CREATEROLE NOSUPERUSER NOBYPASSRLS PASSWORD :migrator_password;

-- 資料庫由 migrator 擁有:citext、btree_gist 是「受信任的擴充」,擁有者就能建立,不需要超級使用者
CREATE DATABASE tenant_saas OWNER tenant_migrator;
