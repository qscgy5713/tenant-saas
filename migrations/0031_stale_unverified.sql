-- 清理從沒驗證過的帳號。
--
-- 沒驗證 Email 的帳號沒辦法建立店家,而且一直占著那個 Email:真正的主人想註冊時只會看到「已註冊」
-- (他能走「忘記密碼」救回來,但這是個陷阱)。超過期限仍沒驗證、也沒有加入任何店家(成員資格)的帳號直接刪除,
-- 連同還沒寄出的信與驗證 / 重設密碼連結。有任何成員資格的不動(那表示有人邀請過、或曾經使用過)。
CREATE FUNCTION retention_delete_stale_unverified(p_days int, p_batch int)
RETURNS int LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_deleted int := 0;
    r         record;
BEGIN
    IF p_days IS NULL OR p_days < 7 THEN
        RAISE EXCEPTION 'retention too short' USING ERRCODE = '22023';
    END IF;
    FOR r IN
        SELECT u.id, u.email::text AS email FROM users u
        WHERE u.email_verified_at IS NULL
          AND u.created_at < now() - make_interval(days => p_days)
          AND NOT EXISTS (SELECT 1 FROM memberships m WHERE m.user_id = u.id)
        ORDER BY u.created_at LIMIT p_batch
        FOR UPDATE OF u SKIP LOCKED
    LOOP
        DELETE FROM email_outbox WHERE lower(to_email) = lower(r.email);
        DELETE FROM users WHERE id = r.id;   -- password_resets / email_verifications 由外鍵串聯刪除
        v_deleted := v_deleted + 1;
    END LOOP;
    RETURN v_deleted;
END
$$;

REVOKE ALL ON FUNCTION retention_delete_stale_unverified(int, int) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION retention_delete_stale_unverified(int, int) TO tenant_worker;
