-- 稽核日誌:只能新增,不能改或刪(對 tenant_app / tenant_worker 都沒有 UPDATE / DELETE 權限)。
-- detail 只放識別碼與欄位名稱等非敏感資訊:不放 Email、電話、token、休假原因。
CREATE TABLE audit_logs (
    id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id     uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    actor_type    text NOT NULL CHECK (actor_type IN ('user', 'customer', 'system')),
    actor_user_id uuid,          -- 不設外鍵:使用者離開後紀錄仍須保留
    action        text NOT NULL,
    entity_type   text NOT NULL,
    entity_id     uuid,
    detail        jsonb NOT NULL DEFAULT '{}',
    created_at    timestamptz NOT NULL DEFAULT now(),
    CHECK ((actor_type = 'user') = (actor_user_id IS NOT NULL))
);
CREATE INDEX audit_logs_tenant_idx ON audit_logs (tenant_id, id DESC);
CREATE INDEX audit_logs_entity_idx ON audit_logs (tenant_id, entity_type, entity_id, id DESC);

-- 只 ENABLE、不 FORCE:create_tenant / accept_invitation(SECURITY DEFINER)需要能在沒有租戶上下文時寫入
ALTER TABLE audit_logs ENABLE ROW LEVEL SECURITY;
CREATE POLICY audit_select ON audit_logs FOR SELECT TO tenant_app USING (tenant_id = app_tenant_id());
CREATE POLICY audit_insert ON audit_logs FOR INSERT TO tenant_app WITH CHECK (tenant_id = app_tenant_id());
CREATE POLICY audit_worker_insert ON audit_logs FOR INSERT TO tenant_worker WITH CHECK (true);
GRANT SELECT, INSERT ON audit_logs TO tenant_app;
GRANT INSERT ON audit_logs TO tenant_worker;

-- 以下兩個函式在沒有租戶上下文時執行,所以稽核要在函式內寫入(與變更同一個交易)
CREATE OR REPLACE FUNCTION create_tenant(p_slug text, p_name text, p_timezone text)
RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_user uuid := app_user_id();
    v_id   uuid;
BEGIN
    IF v_user IS NULL THEN
        RAISE EXCEPTION 'missing user context' USING ERRCODE = '42501';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_timezone_names WHERE name = p_timezone) THEN
        RAISE EXCEPTION 'invalid timezone' USING ERRCODE = '22023';
    END IF;
    INSERT INTO tenants (slug, name, timezone) VALUES (p_slug, p_name, p_timezone)
        RETURNING id INTO v_id;
    INSERT INTO memberships (tenant_id, user_id, role) VALUES (v_id, v_user, 'owner');
    INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
        VALUES (v_id, 'user', v_user, 'tenant.created', 'tenant', v_id, jsonb_build_object('slug', p_slug));
    RETURN v_id;
END
$$;

CREATE OR REPLACE FUNCTION accept_invitation(p_token_hash text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_user  uuid := app_user_id();
    v_inv   invitations%ROWTYPE;
    v_slug  text;
    v_max   int;
    v_count int;
BEGIN
    IF v_user IS NULL THEN
        RAISE EXCEPTION 'missing user context' USING ERRCODE = '42501';
    END IF;

    SELECT i.* INTO v_inv
    FROM invitations i JOIN users u ON u.email = i.email
    WHERE i.token_hash = p_token_hash
      AND u.id = v_user
      AND i.accepted_at IS NULL
      AND i.expires_at > now()
    FOR UPDATE OF i;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'invalid invitation' USING ERRCODE = 'P0002';
    END IF;

    PERFORM pg_advisory_xact_lock(hashtextextended('quota:staff:' || v_inv.tenant_id::text, 0));
    IF NOT EXISTS (SELECT 1 FROM memberships WHERE tenant_id = v_inv.tenant_id AND user_id = v_user) THEN
        SELECT p.max_staff INTO v_max
        FROM tenants t JOIN plans p ON p.id = t.plan_id WHERE t.id = v_inv.tenant_id;
        SELECT count(*) INTO v_count FROM memberships WHERE tenant_id = v_inv.tenant_id;
        IF v_max IS NOT NULL AND v_count >= v_max THEN
            RAISE EXCEPTION 'plan limit reached' USING ERRCODE = '53400';
        END IF;
    END IF;

    INSERT INTO memberships (tenant_id, user_id, role)
    VALUES (v_inv.tenant_id, v_user, v_inv.role)
    ON CONFLICT (tenant_id, user_id) DO NOTHING;

    UPDATE invitations SET accepted_at = now() WHERE id = v_inv.id;

    INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
        VALUES (v_inv.tenant_id, 'user', v_user, 'invitation.accepted', 'invitation', v_inv.id,
                jsonb_build_object('role', v_inv.role));

    SELECT slug INTO v_slug FROM tenants WHERE id = v_inv.tenant_id;
    RETURN v_slug;
END
$$;
