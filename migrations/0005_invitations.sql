CREATE TABLE invitations (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    email       citext NOT NULL,
    role        member_role NOT NULL CHECK (role <> 'owner'),
    token_hash  text NOT NULL UNIQUE,         -- 只存 SHA-256,原始 token 只在建立時回傳一次
    expires_at  timestamptz NOT NULL,
    accepted_at timestamptz,
    created_at  timestamptz NOT NULL DEFAULT now()
);

-- 同一家店對同一個 Email 同時只能有一張待處理邀請
CREATE UNIQUE INDEX invitations_pending_email_idx
    ON invitations (tenant_id, email) WHERE accepted_at IS NULL;

-- 只 ENABLE、不 FORCE:accept_invitation(SECURITY DEFINER)需要能跨租戶查 token
ALTER TABLE invitations ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON invitations
    USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id());
GRANT SELECT, INSERT, DELETE ON invitations TO tenant_app;

-- 被邀請者點連結時還不是成員,沒有租戶上下文,所以用函式處理。
-- 使用者取自請求上下文,且登入者的 Email 必須等於被邀請的 Email(別人拿到連結也不能用)。
-- 無效、過期、已使用、Email 不符一律同一個錯誤,不洩漏原因。
CREATE FUNCTION accept_invitation(p_token_hash text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_user uuid := app_user_id();
    v_inv  invitations%ROWTYPE;
    v_slug text;
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

    INSERT INTO memberships (tenant_id, user_id, role)
    VALUES (v_inv.tenant_id, v_user, v_inv.role)
    ON CONFLICT (tenant_id, user_id) DO NOTHING;

    UPDATE invitations SET accepted_at = now() WHERE id = v_inv.id;

    SELECT slug INTO v_slug FROM tenants WHERE id = v_inv.tenant_id;
    RETURN v_slug;
END
$$;
REVOKE ALL ON FUNCTION accept_invitation(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION accept_invitation(text) TO tenant_app;
