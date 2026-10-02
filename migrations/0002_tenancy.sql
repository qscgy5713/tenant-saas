-- 應用程式在每個租戶交易開頭 SET LOCAL ROLE tenant_app,讓 RLS 對它生效。
-- 角色是整個叢集共用的,平行建立時可能撞到,所以吞掉重複錯誤。
DO $$
BEGIN
    CREATE ROLE tenant_app NOLOGIN NOBYPASSRLS;
EXCEPTION WHEN duplicate_object OR unique_violation THEN
    NULL;
END $$;
GRANT tenant_app TO CURRENT_USER;

-- 目前請求的上下文,未設定時為 NULL(RLS 比對 NULL 等於看不到任何資料)
CREATE FUNCTION app_user_id() RETURNS uuid LANGUAGE sql STABLE AS
$$ SELECT nullif(current_setting('app.user_id', true), '')::uuid $$;

CREATE FUNCTION app_tenant_id() RETURNS uuid LANGUAGE sql STABLE AS
$$ SELECT nullif(current_setting('app.tenant_id', true), '')::uuid $$;

CREATE TYPE member_role AS ENUM ('owner', 'manager', 'staff');

CREATE TABLE tenants (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    slug       text NOT NULL UNIQUE
               CHECK (slug ~ '^[a-z0-9][a-z0-9-]{1,38}[a-z0-9]$'),
    name       text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    timezone   text NOT NULL DEFAULT 'Asia/Taipei',
    status     text NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended')),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE memberships (
    tenant_id  uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id    uuid NOT NULL REFERENCES users(id)   ON DELETE CASCADE,
    role       member_role NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, user_id)
);
CREATE INDEX memberships_user_id_idx ON memberships (user_id);

-- tenants / memberships 只 ENABLE、不 FORCE:SECURITY DEFINER 函式(以擁有者身分執行)需要能寫入
ALTER TABLE tenants     ENABLE ROW LEVEL SECURITY;
ALTER TABLE memberships ENABLE ROW LEVEL SECURITY;

-- 已選定租戶時只看得到該租戶;尚未選定時(列出「我的店家」)只看得到自己所屬的
CREATE POLICY tenants_select ON tenants FOR SELECT USING (
    id = app_tenant_id()
    OR (app_tenant_id() IS NULL
        AND id IN (SELECT tenant_id FROM memberships WHERE user_id = app_user_id()))
);

CREATE POLICY memberships_access ON memberships FOR ALL
    USING (CASE WHEN app_tenant_id() IS NOT NULL
                THEN tenant_id = app_tenant_id()
                ELSE user_id = app_user_id() END)
    WITH CHECK (tenant_id = app_tenant_id());

-- tenant_app 的權限:不給密碼雜湊,tenants 只讀(建立走函式)
GRANT SELECT (id, email, name, created_at) ON users TO tenant_app;
GRANT SELECT ON tenants TO tenant_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON memberships TO tenant_app;

-- 建立店家並把呼叫者設為 owner。使用者取自請求上下文,不接受參數,避免替別人建立。
CREATE FUNCTION create_tenant(p_slug text, p_name text, p_timezone text)
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
    RETURN v_id;
END
$$;
REVOKE ALL ON FUNCTION create_tenant(text, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION create_tenant(text, text, text) TO tenant_app;
