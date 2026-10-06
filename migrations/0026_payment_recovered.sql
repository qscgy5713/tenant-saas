-- 付款恢復通知:先前扣款失敗的那張發票後來付清了,告訴店主「恢復了」。
--
-- 只在「這張發票先前失敗過」時才通知。正常的每月續訂(沒有失敗過)不寄信 —— Stripe 本身可以寄收據
-- (Dashboard 可開),兩邊都寄會讓店主收到兩封。
--
-- 事件順序 Stripe 不保證:`invoice.paid` 可能比 `invoice.payment_failed` 先到(例如重送、延遲)。
-- 所以每筆付清的發票都記下來(last_paid_invoice);之後才到的「失敗」事件發現這張發票已經付清,
-- 就不再寄「扣款失敗」(否則店主會收到一封已經過時、而且會嚇到人的信)。
ALTER TABLE tenant_billing
    ADD COLUMN last_failed_invoice text,
    ADD COLUMN last_paid_invoice   text;

-- 與 0018 相同,多了:記下失敗的發票;這張發票若已經付清(亂序)就不寄信
CREATE OR REPLACE FUNCTION billing_payment_failed(
    p_event_id text, p_type text, p_customer text, p_invoice text,
    p_attempt int, p_next_attempt bigint,
    p_subject text, p_body text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_tenant  tenants%ROWTYPE;
    v_paid    text;
    v_body    text;
    v_owner   record;
BEGIN
    INSERT INTO stripe_events (id, type) VALUES (p_event_id, p_type) ON CONFLICT DO NOTHING;
    IF NOT FOUND THEN
        RETURN 'duplicate';
    END IF;

    SELECT t.* INTO v_tenant
    FROM tenant_billing b JOIN tenants t ON t.id = b.tenant_id
    WHERE b.stripe_customer_id = p_customer
    FOR UPDATE OF b;
    IF NOT FOUND THEN
        RETURN 'unknown_customer';
    END IF;
    SELECT last_paid_invoice INTO v_paid FROM tenant_billing WHERE tenant_id = v_tenant.id;
    IF v_paid IS NOT DISTINCT FROM p_invoice THEN
        RETURN 'already_paid';
    END IF;

    UPDATE tenant_billing SET last_failed_invoice = p_invoice WHERE tenant_id = v_tenant.id;

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

-- 一張發票付清了(invoice.paid)。先前失敗過同一張才通知店主;不管有沒有通知,都記下「這張付清了」。
CREATE FUNCTION billing_payment_recovered(
    p_event_id text, p_type text, p_customer text, p_invoice text,
    p_subject text, p_body text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_tenant tenants%ROWTYPE;
    v_failed text;
    v_body   text;
    v_owner  record;
BEGIN
    INSERT INTO stripe_events (id, type) VALUES (p_event_id, p_type) ON CONFLICT DO NOTHING;
    IF NOT FOUND THEN
        RETURN 'duplicate';
    END IF;

    SELECT t.* INTO v_tenant
    FROM tenant_billing b JOIN tenants t ON t.id = b.tenant_id
    WHERE b.stripe_customer_id = p_customer
    FOR UPDATE OF b;
    IF NOT FOUND THEN
        RETURN 'unknown_customer';
    END IF;
    SELECT last_failed_invoice INTO v_failed FROM tenant_billing WHERE tenant_id = v_tenant.id;

    UPDATE tenant_billing SET last_paid_invoice = p_invoice,
        last_failed_invoice = CASE WHEN last_failed_invoice = p_invoice THEN NULL ELSE last_failed_invoice END
    WHERE tenant_id = v_tenant.id;

    IF v_failed IS DISTINCT FROM p_invoice THEN
        RETURN 'no_prior_failure';
    END IF;

    v_body := replace(replace(p_body, '{shop}', v_tenant.name), '{slug}', v_tenant.slug);
    FOR v_owner IN
        SELECT u.id, u.email::text AS email FROM memberships m JOIN users u ON u.id = m.user_id
        WHERE m.tenant_id = v_tenant.id AND m.role = 'owner'
    LOOP
        INSERT INTO email_outbox (tenant_id, to_email, subject, body, dedupe_key)
        VALUES (v_tenant.id, v_owner.email, replace(p_subject, '{shop}', v_tenant.name), v_body,
                'payment_recovered:' || p_event_id || ':' || v_owner.id)
        ON CONFLICT (dedupe_key) DO NOTHING;
    END LOOP;

    INSERT INTO audit_logs (tenant_id, actor_type, action, entity_type, entity_id, detail)
    VALUES (v_tenant.id, 'system', 'billing.payment_recovered', 'tenant', v_tenant.id, '{}');
    RETURN 'applied';
END
$$;

REVOKE ALL ON FUNCTION billing_payment_recovered(text, text, text, text, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION billing_payment_recovered(text, text, text, text, text, text) TO tenant_runtime;
