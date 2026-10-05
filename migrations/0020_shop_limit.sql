-- 每位使用者可以「擁有」的店家數量上限(防止灌店家、占用代稱)。
--
-- 上限由呼叫端(應用程式設定 MAX_SHOPS_PER_USER)傳入。這是產品層面的防濫用,不是安全邊界:
-- 能直接以 tenant_app 執行 SQL 的人本來就能做更多事。
-- 只算 owner 身分的店;受邀加入別人的店不算。用 advisory lock 依使用者序列化,
-- 否則同時送出多個建店請求可以一起通過檢查。
--
-- 簽章多了一個參數,所以舊的三參數版本必須移除 —— 否則 tenant_app 仍可呼叫它而繞過上限。
DROP FUNCTION create_tenant(text, text, text);

CREATE FUNCTION create_tenant(p_slug text, p_name text, p_timezone text, p_max_owned int)
RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_user  uuid := app_user_id();
    v_id    uuid;
    v_owned int;
BEGIN
    IF v_user IS NULL THEN
        RAISE EXCEPTION 'missing user context' USING ERRCODE = '42501';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_timezone_names WHERE name = p_timezone) THEN
        RAISE EXCEPTION 'invalid timezone' USING ERRCODE = '22023';
    END IF;

    PERFORM pg_advisory_xact_lock(hashtextextended('shops:' || v_user::text, 0));
    SELECT count(*) INTO v_owned FROM memberships WHERE user_id = v_user AND role = 'owner';
    IF v_owned >= p_max_owned THEN
        RAISE EXCEPTION 'shop limit reached' USING ERRCODE = '53400';
    END IF;

    INSERT INTO tenants (slug, name, timezone) VALUES (p_slug, p_name, p_timezone)
        RETURNING id INTO v_id;
    INSERT INTO memberships (tenant_id, user_id, role) VALUES (v_id, v_user, 'owner');
    INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
        VALUES (v_id, 'user', v_user, 'tenant.created', 'tenant', v_id, jsonb_build_object('slug', p_slug));
    RETURN v_id;
END
$$;

REVOKE ALL ON FUNCTION create_tenant(text, text, text, int) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION create_tenant(text, text, text, int) TO tenant_app;
