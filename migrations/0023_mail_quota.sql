-- 每位收件者每小時能收到的信上限(跨所有店家計算)。
--
-- 防的是「拿別人的 Email 當收件者」的騷擾:公開預約頁任何人都能填任意 Email,系統就會寄驗證信給它。
-- 單一店家內已經有「同一 Email 最多 5 筆未來預約」的限制,但攻擊者可以對很多家店各來一次。
--
-- 為什麼是資料庫函式:email_outbox 有 RLS,一般請求只看得到自己店家的信,算不出跨店家的數量。
-- (需要 0022 之後 email_outbox 不 FORCE,擁有者才看得到全部。)
-- 為什麼要加鎖:同一個收件者的並行請求會同時通過「還沒超過」的檢查。以收件者為單位的 advisory lock
-- 持有到交易結束,而呼叫端在同一個交易內接著寫入信件,所以檢查與寫入之間不會被插隊。
CREATE FUNCTION mail_quota_ok(p_email text, p_max int)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_recent int;
BEGIN
    PERFORM pg_advisory_xact_lock(hashtextextended('mailquota:' || lower(p_email), 0));
    SELECT count(*) INTO v_recent FROM email_outbox
    WHERE lower(to_email) = lower(p_email) AND created_at > now() - interval '1 hour';
    RETURN v_recent < p_max;
END
$$;

REVOKE ALL ON FUNCTION mail_quota_ok(text, int) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION mail_quota_ok(text, int) TO tenant_app;
-- 查詢用的索引:依收件者與時間
CREATE INDEX email_outbox_recipient_idx ON email_outbox (lower(to_email), created_at);
