-- 計費(Stripe)。
--
-- 計費狀態放在獨立的 tenant_billing 表,**租戶角色與執行階段帳號都沒有直接權限**:
-- 店家成員不該能讀到 Stripe 的客戶 / 訂閱編號,更不能自己改方案。
-- 只能透過下面三個 SECURITY DEFINER 函式存取:
--   billing_state()          租戶上下文內,讀自己店家的訂閱狀態
--   billing_attach_customer  租戶上下文內,綁定 Stripe 客戶(只綁一次,之後回傳既有的)
--   billing_apply_event      webhook 用(執行階段帳號,沒有租戶上下文)
CREATE TABLE tenant_billing (
    tenant_id            uuid PRIMARY KEY REFERENCES tenants(id) ON DELETE CASCADE,
    stripe_customer_id   text NOT NULL UNIQUE,
    subscription_id      text,
    status               text,                 -- Stripe 訂閱狀態:active / past_due / canceled …
    current_period_end   timestamptz,
    cancel_at_period_end boolean NOT NULL DEFAULT false,
    event_at             bigint NOT NULL DEFAULT 0,  -- 已套用的最新事件的 created(秒),用來忽略亂序的舊事件
    updated_at           timestamptz NOT NULL DEFAULT now()
);

-- webhook 事件去重:Stripe 會重送,同一個事件只處理一次
CREATE TABLE stripe_events (
    id          text PRIMARY KEY,
    type        text NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now()
);

CREATE FUNCTION billing_state()
RETURNS TABLE (stripe_customer_id text, subscription_id text, status text,
               current_period_end timestamptz, cancel_at_period_end boolean)
LANGUAGE sql SECURITY DEFINER STABLE SET search_path = public, pg_temp AS
$$
    SELECT b.stripe_customer_id, b.subscription_id, b.status, b.current_period_end, b.cancel_at_period_end
    FROM tenant_billing b WHERE b.tenant_id = app_tenant_id()
$$;

CREATE FUNCTION billing_attach_customer(p_customer text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_tenant uuid := app_tenant_id();
    v_stored text;
BEGIN
    IF v_tenant IS NULL THEN
        RAISE EXCEPTION 'missing tenant context' USING ERRCODE = '42501';
    END IF;
    INSERT INTO tenant_billing (tenant_id, stripe_customer_id) VALUES (v_tenant, p_customer)
    ON CONFLICT (tenant_id) DO NOTHING;
    SELECT stripe_customer_id INTO v_stored FROM tenant_billing WHERE tenant_id = v_tenant;
    RETURN v_stored;
END
$$;

-- 套用一個訂閱事件。回傳處理結果(給日誌 / 測試):
--   applied / duplicate / stale / unknown_customer
-- p_plan:這個訂閱對應的方案(由呼叫端依 price id 換算);訂閱結束時傳 NULL → 退回免費版
CREATE FUNCTION billing_apply_event(
    p_event_id text, p_type text, p_created bigint, p_customer text,
    p_subscription text, p_status text, p_plan text,
    p_period_end timestamptz, p_cancel_at_period_end boolean)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_billing tenant_billing%ROWTYPE;
    v_new_plan text;
    v_old_plan text;
BEGIN
    INSERT INTO stripe_events (id, type) VALUES (p_event_id, p_type) ON CONFLICT DO NOTHING;
    IF NOT FOUND THEN
        RETURN 'duplicate';
    END IF;

    SELECT * INTO v_billing FROM tenant_billing WHERE stripe_customer_id = p_customer FOR UPDATE;
    IF NOT FOUND THEN
        RETURN 'unknown_customer';
    END IF;
    -- Stripe 不保證事件順序:比已套用的還舊就忽略,否則「已取消」之後晚到的「更新」會讓方案復活
    IF p_created < v_billing.event_at THEN
        RETURN 'stale';
    END IF;

    -- 還在付款流程中的(incomplete)不改方案;付費中 / 寬限中(past_due)維持付費方案;其餘退回免費
    v_new_plan := CASE
        WHEN p_status IN ('active', 'trialing', 'past_due') AND p_plan IS NOT NULL THEN p_plan
        WHEN p_status IN ('incomplete') THEN NULL
        ELSE 'free'
    END;

    UPDATE tenant_billing SET
        subscription_id = p_subscription, status = p_status, current_period_end = p_period_end,
        cancel_at_period_end = p_cancel_at_period_end, event_at = p_created, updated_at = now()
    WHERE tenant_id = v_billing.tenant_id;

    IF v_new_plan IS NOT NULL THEN
        SELECT plan_id INTO v_old_plan FROM tenants WHERE id = v_billing.tenant_id;
        IF v_old_plan IS DISTINCT FROM v_new_plan THEN
            UPDATE tenants SET plan_id = v_new_plan WHERE id = v_billing.tenant_id;
            INSERT INTO audit_logs (tenant_id, actor_type, action, entity_type, entity_id, detail)
            VALUES (v_billing.tenant_id, 'system', 'billing.plan_changed', 'tenant', v_billing.tenant_id,
                    jsonb_build_object('from', v_old_plan, 'to', v_new_plan, 'subscription_status', p_status));
        END IF;
    END IF;
    RETURN 'applied';
END
$$;

REVOKE ALL ON FUNCTION billing_state() FROM PUBLIC;
REVOKE ALL ON FUNCTION billing_attach_customer(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION billing_apply_event(text, text, bigint, text, text, text, text, timestamptz, boolean) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION billing_state(), billing_attach_customer(text) TO tenant_app;
GRANT EXECUTE ON FUNCTION billing_apply_event(text, text, bigint, text, text, text, text, timestamptz, boolean) TO tenant_runtime;
