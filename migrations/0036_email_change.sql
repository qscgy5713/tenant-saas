-- 更換 Email。
--
-- 為什麼需要:註冊時打錯 Email 的人收不到驗證信,原本只能等 30 天被清理(0031)再重新註冊;
-- 已驗證的人換工作 / 換信箱也沒辦法改。
--
-- 流程:要重新輸入密碼(呼叫端檢查,與登入共用鎖定)→ 寄確認信到**新**地址 → 點連結才真的換。
-- 點得到新地址的信 = 證明擁有它,所以換完新地址就是「已驗證」。完成時寄通知到**舊**地址
-- (帳號被盜用時,原主人至少會知道)。
--
-- 換完之後:
--   * 寄到舊地址、還沒用掉的重設密碼 / 驗證連結全部作廢 —— 舊信箱可能已經不是這個人的了
--   * 不登出其他裝置:換 Email 不代表密碼外洩;要的話使用者可以自己按「登出所有裝置」
--   * 待處理的邀請不搬:邀請是寄給「那個地址」的,接受時本來就比對 Email
-- 只能經由下面的 SECURITY DEFINER 函式存取。
CREATE TABLE email_changes (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    new_email  citext NOT NULL,
    token_hash text NOT NULL UNIQUE,          -- 只存 SHA-256,原始 token 只在信裡
    expires_at timestamptz NOT NULL,
    used_at    timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX email_changes_user_idx ON email_changes (user_id, created_at);

-- 帳號活動多一種事件
ALTER TABLE account_events DROP CONSTRAINT account_events_kind_check;
ALTER TABLE account_events ADD CONSTRAINT account_events_kind_check CHECK (kind IN (
    'registered', 'login', 'login_failed', 'locked', 'logout_all',
    'password_reset', 'email_verified', 'email_changed'));

-- 申請更換。回傳:
--   'sent'       已寄確認信到新地址(同一位使用者先前未完成的申請一併作廢,只有最新的連結有效)
--   'same'       和目前的 Email 相同
--   'taken'      新地址已經是別的帳號
--   'throttled'  這位使用者這小時已申請 3 次,或新地址這小時收到的信已達上限(防止拿來騷擾別人的信箱)
--   'deleted'    帳號已刪除(匿名化)
-- 信件內容由呼叫端組好(含原始 token),只在確定要寄時才寫入 outbox。
CREATE FUNCTION request_email_change(
    p_user uuid, p_new_email text, p_token_hash text, p_subject text, p_body text, p_max_mails int)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_email  text;
    v_recent int;
BEGIN
    SELECT email::text INTO v_email FROM users WHERE id = p_user FOR UPDATE;
    IF NOT FOUND OR v_email LIKE '%@anonymized.invalid' THEN
        RETURN 'deleted';
    END IF;
    IF lower(v_email) = lower(p_new_email) THEN
        RETURN 'same';
    END IF;
    IF EXISTS (SELECT 1 FROM users WHERE email = p_new_email::citext) THEN
        RETURN 'taken';
    END IF;
    SELECT count(*) INTO v_recent FROM email_changes
    WHERE user_id = p_user AND created_at > now() - interval '1 hour';
    IF v_recent >= 3 OR NOT mail_quota_ok(p_new_email, p_max_mails) THEN
        RETURN 'throttled';
    END IF;
    UPDATE email_changes SET used_at = now() WHERE user_id = p_user AND used_at IS NULL;
    INSERT INTO email_changes (user_id, new_email, token_hash, expires_at)
    VALUES (p_user, p_new_email, p_token_hash, now() + interval '24 hours');
    INSERT INTO email_outbox (tenant_id, to_email, subject, body)
    VALUES (NULL, p_new_email, p_subject, p_body);
    RETURN 'sent';
END
$$;

-- 用信中的 token 完成更換。不需要登入(常在另一台裝置開信)。
-- token 無效 / 過期 / 已使用 / 帳號已刪除:P0002(同一個錯誤);新地址在這期間被別人註冊走了:P0001。
-- 成功時寄通知到舊地址:p_notice_body 裡的 `{new_email}` 由這裡代入(呼叫端在這之前不知道是哪個帳號)。
-- 回傳 user_id。
CREATE FUNCTION confirm_email_change(p_token_hash text, p_notice_subject text, p_notice_body text)
RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_row email_changes%ROWTYPE;
    v_old text;
BEGIN
    SELECT * INTO v_row FROM email_changes
    WHERE token_hash = p_token_hash AND used_at IS NULL AND expires_at > now()
    FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'invalid email change token' USING ERRCODE = 'P0002';
    END IF;
    SELECT email::text INTO v_old FROM users WHERE id = v_row.user_id FOR UPDATE;
    -- 申請之後才刪除帳號:不能讓舊連結把匿名帳號「復活」成占住新地址的帳號
    IF v_old LIKE '%@anonymized.invalid' THEN
        RAISE EXCEPTION 'account deleted' USING ERRCODE = 'P0002';
    END IF;
    BEGIN
        UPDATE users SET email = v_row.new_email, email_verified_at = now() WHERE id = v_row.user_id;
    EXCEPTION WHEN unique_violation THEN
        RAISE EXCEPTION 'email taken' USING ERRCODE = 'P0001';
    END;
    UPDATE email_changes SET used_at = now() WHERE user_id = v_row.user_id AND used_at IS NULL;
    -- 舊信箱可能已經不是這個人的:寄到那裡、還沒用掉的連結全部作廢
    UPDATE password_resets SET used_at = now() WHERE user_id = v_row.user_id AND used_at IS NULL;
    UPDATE email_verifications SET used_at = now() WHERE user_id = v_row.user_id AND used_at IS NULL;
    INSERT INTO email_outbox (tenant_id, to_email, subject, body)
    VALUES (NULL, v_old, p_notice_subject, replace(p_notice_body, '{new_email}', v_row.new_email::text));
    RETURN v_row.user_id;
END
$$;

REVOKE ALL ON FUNCTION request_email_change(uuid, text, text, text, text, int),
                       confirm_email_change(text, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION request_email_change(uuid, text, text, text, text, int),
                          confirm_email_change(text, text, text) TO tenant_runtime;
