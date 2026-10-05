-- 帳號鎖定:同一個帳號連續登入失敗 5 次,鎖 15 分鐘。
--
-- 依 IP 的限流擋不住「很多 IP 輪流猜同一個帳號」,所以另外在帳號層級計數。
-- 取捨:攻擊者可以故意輸錯把別人鎖住(但不能登入)。緩解:
--   * 鎖定期間的失敗不再延長鎖定時間(不能永遠鎖著)
--   * 「忘記密碼」不受鎖定影響,成功重設密碼會立刻解鎖
--   * 鎖定時寄信通知帳號擁有者
-- 計數只寫在已存在的帳號上;鎖定中的帳號對外仍回一模一樣的 401,不洩漏帳號是否存在 / 是否被鎖。
ALTER TABLE users
    ADD COLUMN failed_logins        int NOT NULL DEFAULT 0,
    ADD COLUMN last_failed_login_at timestamptz,
    ADD COLUMN locked_until         timestamptz;

-- 記一次失敗。回傳 true 表示「這一次」觸發了鎖定(呼叫端據此記錄日誌 / 指標)。
-- 鎖定期間不計數、不延長。距上次失敗超過 15 分鐘,或上次的鎖定已到期,就重新從 1 算起。
-- 信件內容由呼叫端組好,函式只在真的鎖定時才寫入 outbox。
CREATE FUNCTION login_failed(p_user uuid, p_subject text, p_body text)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_u     users%ROWTYPE;
    v_count int;
BEGIN
    SELECT * INTO v_u FROM users WHERE id = p_user FOR UPDATE;
    IF NOT FOUND OR (v_u.locked_until IS NOT NULL AND v_u.locked_until > now()) THEN
        RETURN false;
    END IF;

    IF v_u.last_failed_login_at IS NULL
       OR v_u.last_failed_login_at < now() - interval '15 minutes'
       OR v_u.locked_until IS NOT NULL THEN   -- 走到這裡表示上次的鎖已到期
        v_count := 1;
    ELSE
        v_count := v_u.failed_logins + 1;
    END IF;

    IF v_count >= 5 THEN
        UPDATE users SET failed_logins = v_count, last_failed_login_at = now(),
                         locked_until = now() + interval '15 minutes'
        WHERE id = p_user;
        INSERT INTO email_outbox (tenant_id, to_email, subject, body)
        VALUES (NULL, v_u.email::text, p_subject, p_body);
        RETURN true;
    END IF;

    UPDATE users SET failed_logins = v_count, last_failed_login_at = now(), locked_until = NULL
    WHERE id = p_user;
    RETURN false;
END
$$;

-- 登入成功:清掉失敗計數(沒有計數就不寫,避免每次登入都產生寫入)
CREATE FUNCTION login_succeeded(p_user uuid)
RETURNS void LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
    UPDATE users SET failed_logins = 0, last_failed_login_at = NULL, locked_until = NULL
    WHERE id = p_user AND (failed_logins > 0 OR locked_until IS NOT NULL);
$$;

-- 重設密碼成功要一併解鎖(其餘邏輯與 0013 相同)
CREATE OR REPLACE FUNCTION reset_password(p_token_hash text, p_new_password_hash text)
RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_reset password_resets%ROWTYPE;
BEGIN
    SELECT * INTO v_reset FROM password_resets
    WHERE token_hash = p_token_hash AND used_at IS NULL AND expires_at > now()
    FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'invalid reset token' USING ERRCODE = 'P0002';
    END IF;

    UPDATE users SET password_hash = p_new_password_hash, password_changed_at = now(),
                     failed_logins = 0, last_failed_login_at = NULL, locked_until = NULL
    WHERE id = v_reset.user_id;
    UPDATE password_resets SET used_at = now()
    WHERE user_id = v_reset.user_id AND used_at IS NULL;
    RETURN v_reset.user_id;
END
$$;

REVOKE ALL ON FUNCTION login_failed(uuid, text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION login_succeeded(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION login_failed(uuid, text, text) TO tenant_runtime;
GRANT EXECUTE ON FUNCTION login_succeeded(uuid) TO tenant_runtime;
