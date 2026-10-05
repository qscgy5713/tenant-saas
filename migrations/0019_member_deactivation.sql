-- 員工停用:離職的成員有預約紀錄,不能刪除(外鍵),改成「停用」。
--
-- 停用的成員:不能再存取這間店、不出現在可預約的人員名單、不佔方案的人數名額;
-- 歷史預約與稽核完整保留。可以重新啟用(重新啟用時會檢查名額)。
ALTER TABLE memberships ADD COLUMN active boolean NOT NULL DEFAULT true;

-- 方案人數上限只算啟用中的成員。其餘邏輯與 0009 相同。
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
        SELECT count(*) INTO v_count FROM memberships WHERE tenant_id = v_inv.tenant_id AND active;
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
