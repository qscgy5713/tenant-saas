-- 刪除店家的安全網(CR 發現):申請刪除時檢查過「沒有未來預約」,但之後仍可能出現預約
-- (申請與新增預約同時發生、或將來新增的其他入口)。硬刪除不能因此丟掉顧客還在等的預約:
-- 到期時如果還有未來已確認 / 待確認的預約,**不刪**,改成自動取消這次刪除並留下稽核(原因 live_bookings),
-- 由店主重新決定。
CREATE OR REPLACE FUNCTION retention_delete_tenants(p_batch int)
RETURNS SETOF uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_id   uuid;
    v_live int;
BEGIN
    FOR v_id IN
        SELECT id FROM tenants WHERE deletion_scheduled_at IS NOT NULL AND deletion_scheduled_at <= now()
        ORDER BY deletion_scheduled_at LIMIT p_batch FOR UPDATE SKIP LOCKED
    LOOP
        SELECT count(*) INTO v_live FROM bookings
        WHERE tenant_id = v_id AND (status = 'pending' OR (status = 'confirmed' AND starts_at > now()));
        IF v_live > 0 THEN
            UPDATE tenants SET deletion_requested_at = NULL, deletion_scheduled_at = NULL WHERE id = v_id;
            INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
            VALUES (v_id, 'system', NULL, 'tenant.deletion_cancelled', 'tenant', v_id,
                    jsonb_build_object('reason', 'live_bookings', 'count', v_live));
            CONTINUE;
        END IF;
        -- bookings 對成員 / 服務 / 顧客的外鍵沒有串聯刪除(為了保護有紀錄的資料),
        -- 串聯刪除店家時會卡在這裡,所以先明確刪掉預約
        DELETE FROM bookings WHERE tenant_id = v_id;
        DELETE FROM tenants WHERE id = v_id;
        RETURN NEXT v_id;
    END LOOP;
END
$$;

-- 申請刪除與新增預約互斥(見 booking::create_booking 的共享鎖):先取得獨佔鎖,再檢查有沒有未來預約。
-- 其餘與 0028 相同。
CREATE OR REPLACE FUNCTION request_tenant_deletion(p_grace_days int)
RETURNS timestamptz LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_tenant uuid := app_tenant_id();
    v_user   uuid := app_user_id();
    v_when   timestamptz;
    v_live   int;
BEGIN
    IF v_tenant IS NULL OR v_user IS NULL THEN
        RAISE EXCEPTION 'missing context' USING ERRCODE = '42501';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM memberships WHERE tenant_id = v_tenant AND user_id = v_user AND role = 'owner') THEN
        RAISE EXCEPTION 'owner only' USING ERRCODE = '42501';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('tenant-deletion:' || v_tenant::text, 0));
    PERFORM 1 FROM tenants WHERE id = v_tenant FOR UPDATE;
    IF EXISTS (SELECT 1 FROM tenants WHERE id = v_tenant AND deletion_scheduled_at IS NOT NULL) THEN
        RAISE EXCEPTION 'already requested' USING ERRCODE = 'P0003';
    END IF;
    SELECT count(*) INTO v_live FROM bookings
    WHERE tenant_id = v_tenant AND (status = 'pending' OR (status = 'confirmed' AND starts_at > now()));
    IF v_live > 0 THEN
        RAISE EXCEPTION 'upcoming bookings: %', v_live USING ERRCODE = 'P0001';
    END IF;
    IF EXISTS (SELECT 1 FROM tenant_billing
               WHERE tenant_id = v_tenant AND status IS NOT NULL
                 AND status NOT IN ('canceled', 'incomplete_expired')) THEN
        RAISE EXCEPTION 'active subscription' USING ERRCODE = 'P0002';
    END IF;

    v_when := now() + make_interval(days => p_grace_days);
    UPDATE tenants SET deletion_requested_at = now(), deletion_scheduled_at = v_when WHERE id = v_tenant;
    INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
    VALUES (v_tenant, 'user', v_user, 'tenant.deletion_requested', 'tenant', v_tenant,
            jsonb_build_object('scheduled_at', v_when));
    RETURN v_when;
END
$$;
