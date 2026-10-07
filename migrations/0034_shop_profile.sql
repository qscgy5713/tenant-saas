-- 店家資訊:簡介、地址、電話。顯示在顧客的預約頁上。全部選填;只有店主能改(經函式,留稽核)。
-- 長度限制在資料庫也擋一次(應用程式驗證之外的最後一道防線)。
ALTER TABLE tenants
    ADD COLUMN description text CHECK (char_length(description) BETWEEN 1 AND 500),
    ADD COLUMN address     text CHECK (char_length(address) BETWEEN 1 AND 200),
    ADD COLUMN phone       text CHECK (char_length(phone) BETWEEN 1 AND 30);

-- 整組取代(不是只改有帶的欄位):空字串 / NULL = 清掉。稽核只記「哪些欄位變了」,不記內容
CREATE FUNCTION update_shop_profile(p_description text, p_address text, p_phone text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_tenant uuid := app_tenant_id();
    v_user   uuid := app_user_id();
    v_old    tenants%ROWTYPE;
    v_d text := NULLIF(btrim(p_description), '');
    v_a text := NULLIF(btrim(p_address), '');
    v_p text := NULLIF(btrim(p_phone), '');
BEGIN
    IF v_tenant IS NULL OR v_user IS NULL THEN
        RAISE EXCEPTION 'missing context' USING ERRCODE = '42501';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM memberships WHERE tenant_id = v_tenant AND user_id = v_user AND role = 'owner') THEN
        RAISE EXCEPTION 'owner only' USING ERRCODE = '42501';
    END IF;
    SELECT * INTO v_old FROM tenants WHERE id = v_tenant FOR UPDATE;
    UPDATE tenants SET description = v_d, address = v_a, phone = v_p WHERE id = v_tenant;
    INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
    VALUES (v_tenant, 'user', v_user, 'tenant.profile_updated', 'tenant', v_tenant,
            jsonb_build_object('changed', to_jsonb(array_remove(ARRAY[
                CASE WHEN v_d IS DISTINCT FROM v_old.description THEN 'description' END,
                CASE WHEN v_a IS DISTINCT FROM v_old.address THEN 'address' END,
                CASE WHEN v_p IS DISTINCT FROM v_old.phone THEN 'phone' END], NULL))));
END
$$;
REVOKE ALL ON FUNCTION update_shop_profile(text, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION update_shop_profile(text, text, text) TO tenant_app;
