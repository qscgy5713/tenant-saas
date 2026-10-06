-- 使用者帳號刪除(本人要求)。
--
-- 不是刪除 users 的資料列:成員資格、預約(員工)、稽核(actor_user_id)都指著它,而且那些紀錄要保留。
-- 做法與「顧客匿名化」相同:把「這個人是誰」抹掉 —— Email 換成無法還原的代號(之後這個 Email 可以重新註冊)、
-- 名稱換成「已刪除的使用者」、密碼改成不可能驗證通過的值、所有成員資格停用、所有登入立刻失效,
-- 寄給他的待寄信件刪除、已寄出的紀錄與邀請上的 Email 改成匿名地址。
--
-- 不能刪除的情況(顧客或店家會因此出問題):
--   P0002:還是某家(沒在申請刪除的)店的擁有者 —— 先刪除店家(目前沒有移交所有權的功能)
--   P0001:還有負責的未來 / 待確認預約 —— 先請店家改派或取消
--   P0003:已經刪除過了
CREATE FUNCTION delete_account(p_user uuid)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_email text;
    v_anon  text;
    r       record;
BEGIN
    SELECT email::text INTO v_email FROM users WHERE id = p_user FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'unknown user' USING ERRCODE = 'P0004';
    END IF;
    IF v_email LIKE '%@anonymized.invalid' THEN
        RAISE EXCEPTION 'already deleted' USING ERRCODE = 'P0003';
    END IF;
    IF EXISTS (SELECT 1 FROM memberships m JOIN tenants t ON t.id = m.tenant_id
               WHERE m.user_id = p_user AND m.role = 'owner' AND m.active
                 AND t.deletion_scheduled_at IS NULL) THEN
        RAISE EXCEPTION 'owns shops' USING ERRCODE = 'P0002';
    END IF;
    IF EXISTS (SELECT 1 FROM bookings
               WHERE staff_user_id = p_user
                 AND (status = 'pending' OR (status = 'confirmed' AND starts_at > now()))) THEN
        RAISE EXCEPTION 'upcoming bookings' USING ERRCODE = 'P0001';
    END IF;

    v_anon := 'deleted-' || p_user::text || '@anonymized.invalid';
    UPDATE users
    SET email = v_anon, name = '已刪除的使用者', password_hash = '!deleted',
        sessions_revoked_at = now(), password_changed_at = now(),
        failed_logins = 0, last_failed_login_at = NULL, locked_until = NULL
    WHERE id = p_user;

    -- 每家店留一筆稽核(只有 id,沒有個資)
    FOR r IN SELECT tenant_id FROM memberships WHERE user_id = p_user LOOP
        INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
        VALUES (r.tenant_id, 'user', p_user, 'member.account_deleted', 'member', p_user, '{}');
    END LOOP;
    UPDATE memberships SET active = false WHERE user_id = p_user;

    DELETE FROM password_resets WHERE user_id = p_user;
    DELETE FROM email_verifications WHERE user_id = p_user;
    DELETE FROM invitations WHERE lower(email::text) = lower(v_email) AND accepted_at IS NULL;
    UPDATE invitations SET email = v_anon WHERE lower(email::text) = lower(v_email);

    DELETE FROM email_outbox WHERE status = 'pending' AND lower(to_email) = lower(v_email);
    UPDATE email_outbox SET to_email = v_anon WHERE lower(to_email) = lower(v_email);
END
$$;

REVOKE ALL ON FUNCTION delete_account(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION delete_account(uuid) TO tenant_runtime;
