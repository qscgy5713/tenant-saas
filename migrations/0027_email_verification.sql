-- Email 驗證與「登出所有裝置」。
--
-- 1. users.email_verified_at:沒驗證就不能開店(防止拿別人的 Email 註冊、占用網址代稱、
--    以系統名義對顧客寄信)。既有使用者視為已驗證(用註冊時間回填):這是上線前的決定,
--    他們註冊時沒有驗證流程,強迫補驗只會把現有使用者擋在門外。
--    驗證的證據有三種:點了驗證信的連結、點了重設密碼信的連結、接受寄到該 Email 的邀請。
-- 2. users.sessions_revoked_at:「登出所有裝置」。驗證登入時除了 password_changed_at,
--    也檢查簽發時間是否早於這個時間(見 AuthUser)。
-- 3. email_verifications 只能經由下面的 SECURITY DEFINER 函式存取。
ALTER TABLE users ADD COLUMN email_verified_at timestamptz;
UPDATE users SET email_verified_at = created_at;
ALTER TABLE users ADD COLUMN sessions_revoked_at timestamptz;

CREATE TABLE email_verifications (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash text NOT NULL UNIQUE,          -- 只存 SHA-256,原始 token 只在信裡
    expires_at timestamptz NOT NULL,
    used_at    timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX email_verifications_user_idx ON email_verifications (user_id, created_at);

-- 寄驗證信。回傳 'sent' / 'already_verified' / 'throttled'(每位使用者每小時最多 3 封,
-- 避免被拿來灌爆別人的信箱)。信件內容由呼叫端組好(含原始 token),只在確定要寄時才寫入 outbox。
CREATE FUNCTION request_email_verification(p_user uuid, p_token_hash text, p_subject text, p_body text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_email    text;
    v_verified timestamptz;
    v_recent   int;
BEGIN
    SELECT email::text, email_verified_at INTO v_email, v_verified FROM users WHERE id = p_user FOR UPDATE;
    IF NOT FOUND THEN
        RETURN 'unknown_user';
    END IF;
    IF v_verified IS NOT NULL THEN
        RETURN 'already_verified';
    END IF;
    SELECT count(*) INTO v_recent FROM email_verifications
    WHERE user_id = p_user AND created_at > now() - interval '1 hour';
    IF v_recent >= 3 THEN
        RETURN 'throttled';
    END IF;
    INSERT INTO email_verifications (user_id, token_hash, expires_at)
    VALUES (p_user, p_token_hash, now() + interval '24 hours');
    INSERT INTO email_outbox (tenant_id, to_email, subject, body)
    VALUES (NULL, v_email, p_subject, p_body);
    RETURN 'sent';
END
$$;

-- 用 token 驗證。token 無效 / 過期 / 已使用都是同一個錯誤(P0002)。
-- 成功後這位使用者所有未使用的驗證連結一併作廢。
CREATE FUNCTION verify_email(p_token_hash text)
RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_row email_verifications%ROWTYPE;
BEGIN
    SELECT * INTO v_row FROM email_verifications
    WHERE token_hash = p_token_hash AND used_at IS NULL AND expires_at > now()
    FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'invalid verification token' USING ERRCODE = 'P0002';
    END IF;
    UPDATE users SET email_verified_at = COALESCE(email_verified_at, now()) WHERE id = v_row.user_id;
    UPDATE email_verifications SET used_at = now() WHERE user_id = v_row.user_id AND used_at IS NULL;
    RETURN v_row.user_id;
END
$$;

-- 登出所有裝置:此刻之前簽發的 token 全部失效
CREATE FUNCTION revoke_sessions(p_user uuid)
RETURNS void LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
    UPDATE users SET sessions_revoked_at = now() WHERE id = p_user;
$$;

-- 重設密碼的連結也是寄到該信箱,點得到就等於證明擁有它(其餘與 0017 相同:一併解鎖)
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
                     failed_logins = 0, last_failed_login_at = NULL, locked_until = NULL,
                     email_verified_at = COALESCE(email_verified_at, now())
    WHERE id = v_reset.user_id;
    UPDATE password_resets SET used_at = now()
    WHERE user_id = v_reset.user_id AND used_at IS NULL;
    RETURN v_reset.user_id;
END
$$;

-- 接受邀請 = 證明擁有被邀請的 Email(函式只認「Email 與邀請相同」的使用者)。其餘與 0019 相同。
CREATE OR REPLACE FUNCTION accept_invitation(p_token_hash text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_user  uuid := app_user_id();
    v_inv   invitations%ROWTYPE;
    v_slug  text;
    v_max   int;
    v_count int;
BEGIN
    IF v_user IS NULL THEN
        RAISE EXCEPTION 'missing user context' USING ERRCODE = '42501';
    END IF;

    SELECT i.* INTO v_inv
    FROM invitations i JOIN users u ON u.email = i.email
    WHERE i.token_hash = p_token_hash
      AND u.id = v_user
      AND i.accepted_at IS NULL
      AND i.expires_at > now()
    FOR UPDATE OF i;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'invalid invitation' USING ERRCODE = 'P0002';
    END IF;

    PERFORM pg_advisory_xact_lock(hashtextextended('quota:staff:' || v_inv.tenant_id::text, 0));
    IF NOT EXISTS (SELECT 1 FROM memberships WHERE tenant_id = v_inv.tenant_id AND user_id = v_user) THEN
        SELECT p.max_staff INTO v_max
        FROM tenants t JOIN plans p ON p.id = t.plan_id WHERE t.id = v_inv.tenant_id;
        SELECT count(*) INTO v_count FROM memberships WHERE tenant_id = v_inv.tenant_id AND active;
        IF v_max IS NOT NULL AND v_count >= v_max THEN
            RAISE EXCEPTION 'plan limit reached' USING ERRCODE = '53400';
        END IF;
    END IF;

    INSERT INTO memberships (tenant_id, user_id, role)
    VALUES (v_inv.tenant_id, v_user, v_inv.role)
    ON CONFLICT (tenant_id, user_id) DO NOTHING;

    UPDATE invitations SET accepted_at = now() WHERE id = v_inv.id;
    UPDATE users SET email_verified_at = now() WHERE id = v_user AND email_verified_at IS NULL;

    INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
        VALUES (v_inv.tenant_id, 'user', v_user, 'invitation.accepted', 'invitation', v_inv.id,
                jsonb_build_object('role', v_inv.role));

    SELECT slug INTO v_slug FROM tenants WHERE id = v_inv.tenant_id;
    RETURN v_slug;
END
$$;

REVOKE ALL ON FUNCTION request_email_verification(uuid, text, text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION verify_email(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION revoke_sessions(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION request_email_verification(uuid, text, text, text) TO tenant_runtime;
GRANT EXECUTE ON FUNCTION verify_email(text) TO tenant_runtime;
GRANT EXECUTE ON FUNCTION revoke_sessions(uuid) TO tenant_runtime;
