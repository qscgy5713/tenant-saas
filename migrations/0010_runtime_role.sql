-- 執行階段(應用程式)使用的登入帳號。
--
-- 設計:這個帳號 NOINHERIT,本身除了 users / plans 之外沒有任何資料表權限;
-- 每個請求都得先 SET LOCAL ROLE tenant_app(或 tenant_worker)才碰得到租戶資料。
-- 結果是:程式哪天漏了經過 begin_scoped 直接查連線池,會得到 permission denied,而不是悄悄讀到別家店的資料。
--
-- 這裡只建立角色(NOLOGIN)。要讓它能登入,由營運人員另外設定密碼,見 docs/deployment.md:
--   ALTER ROLE tenant_runtime WITH LOGIN PASSWORD '...';
-- 因為 migration 不該保管密碼,而且開發環境用超級使用者連線,不需要這個帳號可登入。

-- 角色是整個叢集共用的,平行建立資料庫時 DDL 會互相踩到。
-- 注意:advisory lock 只在同一個資料庫內有效,擋不住「各自建資料庫」的並行;真正防撞的是下面吞掉重複錯誤。
SELECT pg_advisory_xact_lock(7002001);

-- 角色是整個叢集共用的:平行建立(例如 CI 同時跑很多測試、各自建資料庫)時「先檢查再建立」
-- 會被別人搶先而撞到唯一鍵,所以直接建立並吞掉重複錯誤(與 0002 / 0007 相同)。
DO $$
BEGIN
    CREATE ROLE tenant_runtime NOLOGIN NOINHERIT NOSUPERUSER NOBYPASSRLS;
EXCEPTION WHEN duplicate_object OR unique_violation THEN
    NULL;
END $$;

-- INHERIT FALSE:不自動繼承權限;SET TRUE:允許 SET ROLE。需要 PostgreSQL 16 以上
GRANT tenant_app    TO tenant_runtime WITH INHERIT FALSE, SET TRUE;
GRANT tenant_worker TO tenant_runtime WITH INHERIT FALSE, SET TRUE;

-- 唯一需要直接(不切換角色)存取的資料:註冊 / 登入的 users,以及公開的方案列表
GRANT SELECT, INSERT ON users TO tenant_runtime;
GRANT SELECT ON plans TO tenant_runtime;
