-- 顧客要求刪除個資時,信件佇列裡寄給他的紀錄也要處理。
--
-- email_outbox 對一般請求(tenant_app)只授權 SELECT / INSERT,沒有 UPDATE / DELETE,
-- 所以用一個最小功能的 SECURITY DEFINER 函式:
--   * 還沒寄出的信直接刪除(不能在顧客要求刪除之後還寄信給他)
--   * 已寄出的紀錄改成匿名地址(紀錄保留給「某天寄了幾封信」之類的統計,但不再指向真人)
-- 只作用在目前租戶、只比對指定的那個收件者。需要租戶上下文。
CREATE FUNCTION outbox_forget_recipient(p_old_email text, p_new_email text)
RETURNS int LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_tenant  uuid := app_tenant_id();
    v_deleted int;
BEGIN
    IF v_tenant IS NULL THEN
        RAISE EXCEPTION 'missing tenant context' USING ERRCODE = '42501';
    END IF;
    DELETE FROM email_outbox
    WHERE tenant_id = v_tenant AND status = 'pending' AND lower(to_email) = lower(p_old_email);
    GET DIAGNOSTICS v_deleted = ROW_COUNT;
    UPDATE email_outbox SET to_email = p_new_email
    WHERE tenant_id = v_tenant AND lower(to_email) = lower(p_old_email);
    RETURN v_deleted;
END
$$;

REVOKE ALL ON FUNCTION outbox_forget_recipient(text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION outbox_forget_recipient(text, text) TO tenant_app;
