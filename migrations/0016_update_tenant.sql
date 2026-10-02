-- 店家名稱 / 時區的修改。tenant_app 對 tenants 只有 SELECT(避免店家自己改 plan_id 等欄位),
-- 所以只能經由這個函式,而且函式只動 name 與 timezone,並限定店主。
-- 改時區只改「解讀方式」:既有預約與休假是絕對時間(timestamptz)不變;每週營業時間存的是店家當地時刻,
-- 會跟著新時區重新解讀(前端有提醒)。
CREATE FUNCTION update_tenant(p_name text, p_timezone text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_tenant uuid := app_tenant_id();
    v_user   uuid := app_user_id();
    v_old    tenants%ROWTYPE;
BEGIN
    IF v_tenant IS NULL OR v_user IS NULL THEN
        RAISE EXCEPTION 'missing context' USING ERRCODE = '42501';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM memberships WHERE tenant_id = v_tenant AND user_id = v_user AND role = 'owner') THEN
        RAISE EXCEPTION 'owner only' USING ERRCODE = '42501';
    END IF;
    IF p_timezone IS NOT NULL AND NOT EXISTS (SELECT 1 FROM pg_timezone_names WHERE name = p_timezone) THEN
        RAISE EXCEPTION 'invalid timezone' USING ERRCODE = '22023';
    END IF;

    SELECT * INTO v_old FROM tenants WHERE id = v_tenant FOR UPDATE;
    UPDATE tenants SET name = COALESCE(p_name, name), timezone = COALESCE(p_timezone, timezone)
    WHERE id = v_tenant;

    INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
    VALUES (v_tenant, 'user', v_user, 'tenant.updated', 'tenant', v_tenant,
            jsonb_strip_nulls(jsonb_build_object(
                'name_changed', p_name IS NOT NULL AND p_name IS DISTINCT FROM v_old.name,
                'timezone_from', CASE WHEN p_timezone IS DISTINCT FROM v_old.timezone AND p_timezone IS NOT NULL THEN v_old.timezone END,
                'timezone_to',   CASE WHEN p_timezone IS DISTINCT FROM v_old.timezone THEN p_timezone END)));
END
$$;
REVOKE ALL ON FUNCTION update_tenant(text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION update_tenant(text, text) TO tenant_app;
