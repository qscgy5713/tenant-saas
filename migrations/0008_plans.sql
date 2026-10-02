-- 訂閱方案。NULL 表示不限。
-- tenant_app 只有 SELECT:店家無法自行改方案或改限額,升降級只能由計費流程(之後的 Stripe webhook)或營運人員處理。
CREATE TABLE plans (
    id                     text PRIMARY KEY,
    name                   text NOT NULL,
    price_cents            int  NOT NULL DEFAULT 0 CHECK (price_cents >= 0),
    max_staff              int  CHECK (max_staff IS NULL OR max_staff >= 1),
    max_services           int  CHECK (max_services IS NULL OR max_services >= 0),
    max_bookings_per_month int  CHECK (max_bookings_per_month IS NULL OR max_bookings_per_month >= 0)
);

INSERT INTO plans (id, name, price_cents, max_staff, max_services, max_bookings_per_month) VALUES
    ('free',     '免費版',   0,     2,    5,    50),
    ('pro',      '專業版',   49900, 10,   50,   1000),
    ('business', '企業版',   199900, NULL, NULL, NULL);

ALTER TABLE tenants ADD COLUMN plan_id text NOT NULL DEFAULT 'free' REFERENCES plans(id);

GRANT SELECT ON plans TO tenant_app, tenant_worker;

-- 接受邀請時也要檢查人數上限(邀請建立時預留了名額,但方案可能在這段期間被降級)。
-- 與建立邀請共用同一把 advisory lock,避免兩邊同時通過檢查。
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
            -- 53400 configuration_limit_exceeded
            RAISE EXCEPTION 'plan limit reached' USING ERRCODE = '53400';
        END IF;
    END IF;

    INSERT INTO memberships (tenant_id, user_id, role)
    VALUES (v_inv.tenant_id, v_user, v_inv.role)
    ON CONFLICT (tenant_id, user_id) DO NOTHING;

    UPDATE invitations SET accepted_at = now() WHERE id = v_inv.id;

    SELECT slug INTO v_slug FROM tenants WHERE id = v_inv.tenant_id;
    RETURN v_slug;
END
$$;
