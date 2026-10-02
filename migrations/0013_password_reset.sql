-- 忘記密碼。
--
-- 1. users.password_changed_at:JWT 無法撤銷,所以驗證登入時檢查「簽發時間是否早於上次改密碼」,
--    早於就拒絕。重設密碼後,舊的(可能被盜的)登入立刻失效。
-- 2. email_outbox.tenant_id 改為可空:密碼重設信不屬於任何店家。
--    租戶角色看不到沒有 tenant_id 的列(RLS 政策是 tenant_id = app_tenant_id(),NULL 不成立);
--    只有 worker 的政策是 true,所以只有 worker 能寄出它們。
-- 3. password_resets 只能經由下面兩個 SECURITY DEFINER 函式存取:執行階段帳號沒有任何直接權限。
ALTER TABLE users ADD COLUMN password_changed_at timestamptz;
ALTER TABLE email_outbox ALTER COLUMN tenant_id DROP NOT NULL;

CREATE TABLE password_resets (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash text NOT NULL UNIQUE,          -- 只存 SHA-256,原始 token 只在信裡
    expires_at timestamptz NOT NULL,
    used_at    timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX password_resets_user_idx ON password_resets (user_id, created_at);

-- 申請重設:Email 不存在、或這小時已經申請過 3 次,都「靜默略過」並回傳 false;
-- 呼叫端不論結果都回一樣的回應,不洩漏哪些 Email 已註冊。
-- 信件內容由呼叫端組好(含原始 token)傳進來,函式只在確定要寄時才寫入 outbox。
CREATE FUNCTION request_password_reset(p_email citext, p_token_hash text, p_subject text, p_body text)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_user   uuid;
    v_email  text;
    v_recent int;
BEGIN
    SELECT id, email::text INTO v_user, v_email FROM users WHERE email = p_email;
    IF v_user IS NULL THEN
        RETURN false;
    END IF;

    SELECT count(*) INTO v_recent FROM password_resets
    WHERE user_id = v_user AND created_at > now() - interval '1 hour';
    IF v_recent >= 3 THEN
        RETURN false;
    END IF;

    INSERT INTO password_resets (user_id, token_hash, expires_at)
    VALUES (v_user, p_token_hash, now() + interval '1 hour');
    -- 寄到資料庫裡登記的 Email(不是請求裡的寫法)
    INSERT INTO email_outbox (tenant_id, to_email, subject, body)
    VALUES (NULL, v_email, p_subject, p_body);
    RETURN true;
END
$$;

-- 用 token 重設密碼。token 無效 / 過期 / 已使用都是同一個錯誤(P0002)。
-- 成功後這位使用者所有未使用的重設連結一併作廢,並記下改密碼的時間(讓舊登入失效)。
CREATE FUNCTION reset_password(p_token_hash text, p_new_password_hash text)
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

    UPDATE users SET password_hash = p_new_password_hash, password_changed_at = now()
    WHERE id = v_reset.user_id;
    UPDATE password_resets SET used_at = now()
    WHERE user_id = v_reset.user_id AND used_at IS NULL;
    RETURN v_reset.user_id;
END
$$;

REVOKE ALL ON FUNCTION request_password_reset(citext, text, text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION reset_password(text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION request_password_reset(citext, text, text, text) TO tenant_runtime;
GRANT EXECUTE ON FUNCTION reset_password(text, text) TO tenant_runtime;
