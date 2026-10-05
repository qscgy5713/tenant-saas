-- 付款失敗通知:Stripe 的 invoice.payment_failed 事件 → 寄信給該店的所有店主,並記稽核。
--
-- 與 billing_apply_event 一樣由 webhook(執行階段帳號,沒有租戶上下文)呼叫,
-- 所以是 SECURITY DEFINER:要跨過 RLS 找到店主、寫入該店的 outbox 與稽核。
-- 信件內容由呼叫端組好,其中 {shop} {slug} {next_date} 由這裡代入(店名與時區只有資料庫知道)。
-- 事件去重沿用 stripe_events:Stripe 重送同一個事件只會寄一次。
--
-- 這個函式**不改方案、不動 event_at**:方案由訂閱事件決定(付款失敗時訂閱會進入 past_due,
-- 維持付費方案直到 Stripe 重試用盡才取消),信件只是通知。
CREATE FUNCTION billing_payment_failed(
    p_event_id text, p_type text, p_customer text, p_invoice text,
    p_attempt int, p_next_attempt bigint,
    p_subject text, p_body text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_tenant tenants%ROWTYPE;
    v_body   text;
    v_owner  record;
BEGIN
    INSERT INTO stripe_events (id, type) VALUES (p_event_id, p_type) ON CONFLICT DO NOTHING;
    IF NOT FOUND THEN
        RETURN 'duplicate';
    END IF;

    SELECT t.* INTO v_tenant FROM tenant_billing b JOIN tenants t ON t.id = b.tenant_id
    WHERE b.stripe_customer_id = p_customer;
    IF NOT FOUND THEN
        RETURN 'unknown_customer';
    END IF;

    v_body := replace(replace(p_body, '{shop}', v_tenant.name), '{slug}', v_tenant.slug);
    v_body := replace(v_body, '{next_date}',
        COALESCE(to_char(to_timestamp(p_next_attempt) AT TIME ZONE v_tenant.timezone, 'YYYY-MM-DD'), ''));

    FOR v_owner IN
        SELECT u.id, u.email::text AS email FROM memberships m JOIN users u ON u.id = m.user_id
        WHERE m.tenant_id = v_tenant.id AND m.role = 'owner'
    LOOP
        INSERT INTO email_outbox (tenant_id, to_email, subject, body, dedupe_key)
        VALUES (v_tenant.id, v_owner.email, replace(p_subject, '{shop}', v_tenant.name), v_body,
                'payment_failed:' || p_event_id || ':' || v_owner.id)
        ON CONFLICT (dedupe_key) DO NOTHING;
    END LOOP;

    INSERT INTO audit_logs (tenant_id, actor_type, action, entity_type, entity_id, detail)
    VALUES (v_tenant.id, 'system', 'billing.payment_failed', 'tenant', v_tenant.id,
            jsonb_build_object('attempt', p_attempt, 'final', p_next_attempt IS NULL));
    RETURN 'applied';
END
$$;

REVOKE ALL ON FUNCTION billing_payment_failed(text, text, text, text, int, bigint, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION billing_payment_failed(text, text, text, text, int, bigint, text, text) TO tenant_runtime;
