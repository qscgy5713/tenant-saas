-- 帳號層級的安全事件(登入、失敗、鎖定、登出所有裝置、改密碼、驗證 Email…)。
--
-- 稽核日誌(audit_logs)屬於店家,帳號層級的事件沒有歸屬,所以另開一張不帶租戶的表,
-- 只給使用者本人看(「最近的帳號活動」):發現不是自己的登入或失敗嘗試,就知道該改密碼 / 登出所有裝置。
-- 刻意**不記 IP 與裝置資訊**:那是個資,而且放在反向代理後面不一定準;只記「發生了什麼、什麼時候」。
-- 只能經由 SECURITY DEFINER 函式存取。保留 180 天(背景任務清除)。
CREATE TABLE account_events (
    id         bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind       text NOT NULL CHECK (kind IN (
                   'registered', 'login', 'login_failed', 'locked', 'logout_all',
                   'password_reset', 'email_verified')),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX account_events_user_idx ON account_events (user_id, id DESC);

CREATE FUNCTION record_account_event(p_user uuid, p_kind text)
RETURNS void LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS
$$ INSERT INTO account_events (user_id, kind) VALUES (p_user, p_kind) $$;

CREATE FUNCTION list_account_events(p_user uuid, p_limit int)
RETURNS TABLE (kind text, created_at timestamptz) LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS
$$ SELECT e.kind, e.created_at FROM account_events e WHERE e.user_id = p_user
   ORDER BY e.id DESC LIMIT LEAST(GREATEST(p_limit, 1), 100) $$;

-- 清除過期的事件(背景任務)。下限 30 天
CREATE FUNCTION retention_purge_account_events(p_days int, p_batch int)
RETURNS int LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_n int;
BEGIN
    IF p_days IS NULL OR p_days < 30 THEN
        RAISE EXCEPTION 'retention too short' USING ERRCODE = '22023';
    END IF;
    WITH doomed AS (
        SELECT id FROM account_events WHERE created_at < now() - make_interval(days => p_days)
        ORDER BY id LIMIT p_batch
    )
    DELETE FROM account_events e USING doomed d WHERE e.id = d.id;
    GET DIAGNOSTICS v_n = ROW_COUNT;
    RETURN v_n;
END
$$;

-- 刪除帳號時一併清掉(事件本身沒有個資,但沒有理由留著)。其餘與 0030 相同。
CREATE OR REPLACE FUNCTION delete_account(p_user uuid)
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

    FOR r IN SELECT tenant_id FROM memberships WHERE user_id = p_user LOOP
        INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
        VALUES (r.tenant_id, 'user', p_user, 'member.account_deleted', 'member', p_user, '{}');
    END LOOP;
    UPDATE memberships SET active = false WHERE user_id = p_user;

    DELETE FROM password_resets WHERE user_id = p_user;
    DELETE FROM email_verifications WHERE user_id = p_user;
    DELETE FROM account_events WHERE user_id = p_user;
    DELETE FROM invitations WHERE lower(email::text) = lower(v_email) AND accepted_at IS NULL;
    UPDATE invitations SET email = v_anon WHERE lower(email::text) = lower(v_email);

    DELETE FROM email_outbox WHERE status = 'pending' AND lower(to_email) = lower(v_email);
    UPDATE email_outbox SET to_email = v_anon WHERE lower(to_email) = lower(v_email);
END
$$;

REVOKE ALL ON FUNCTION record_account_event(uuid, text), list_account_events(uuid, int) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION record_account_event(uuid, text), list_account_events(uuid, int) TO tenant_runtime;
REVOKE ALL ON FUNCTION retention_purge_account_events(int, int) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION retention_purge_account_events(int, int) TO tenant_worker;
