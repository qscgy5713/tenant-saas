-- 資料保留與刪除。
--
-- 1. tenants.customer_retention_days:顧客個資在「最後一筆預約之後」保留多久,到期由背景任務匿名化
--    (預設 730 天;店主可調 90–3650)。沒有預約的顧客以建立時間起算。
-- 2. tenants.deletion_requested_at / deletion_scheduled_at:店主申請刪除店家。
--    寬限期內店家只是「關閉公開預約」,後台照常可用(店主可以取消);期滿由背景任務硬刪除。
-- 3. 背景任務用的 SECURITY DEFINER 函式(只授權給 tenant_worker):
--    匿名化單一顧客、清除過期稽核日誌、刪除到期的店家。

ALTER TABLE tenants
    ADD COLUMN customer_retention_days int NOT NULL DEFAULT 730
        CHECK (customer_retention_days BETWEEN 90 AND 3650),
    ADD COLUMN deletion_requested_at timestamptz,
    ADD COLUMN deletion_scheduled_at timestamptz,
    ADD CONSTRAINT tenants_deletion_pair CHECK ((deletion_requested_at IS NULL) = (deletion_scheduled_at IS NULL));

-- 申請刪除的店家:公開預約頁與顧客的預約管理連結都關閉(後台不受影響,店主要能取消)
CREATE OR REPLACE FUNCTION tenant_id_by_slug(p_slug text)
RETURNS uuid LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS
$$ SELECT id FROM tenants WHERE slug = p_slug AND status = 'active' AND deletion_scheduled_at IS NULL $$;

CREATE OR REPLACE FUNCTION booking_tenant_by_token(p_token_hash text)
RETURNS uuid LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS
$$ SELECT b.tenant_id FROM bookings b JOIN tenants t ON t.id = b.tenant_id
   WHERE (b.manage_token_hash = p_token_hash OR b.reminder_token_hash = p_token_hash)
     AND t.status = 'active' AND t.deletion_scheduled_at IS NULL $$;

-- ---------- 店主操作(需要租戶與使用者上下文,只有 owner) ----------

CREATE FUNCTION set_customer_retention(p_days int)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_tenant uuid := app_tenant_id();
    v_user   uuid := app_user_id();
    v_old    int;
BEGIN
    IF v_tenant IS NULL OR v_user IS NULL THEN
        RAISE EXCEPTION 'missing context' USING ERRCODE = '42501';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM memberships WHERE tenant_id = v_tenant AND user_id = v_user AND role = 'owner') THEN
        RAISE EXCEPTION 'owner only' USING ERRCODE = '42501';
    END IF;
    IF p_days IS NULL OR p_days NOT BETWEEN 90 AND 3650 THEN
        RAISE EXCEPTION 'invalid retention' USING ERRCODE = '22023';
    END IF;
    SELECT customer_retention_days INTO v_old FROM tenants WHERE id = v_tenant FOR UPDATE;
    UPDATE tenants SET customer_retention_days = p_days WHERE id = v_tenant;
    INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
    VALUES (v_tenant, 'user', v_user, 'tenant.retention_changed', 'tenant', v_tenant,
            jsonb_build_object('from', v_old, 'to', p_days));
END
$$;

-- 申請刪除。回傳預定刪除的時間。
--   P0001:還有未來已確認或待確認的預約(顧客還在等,先取消)
--   P0002:還有進行中的訂閱(先取消訂閱,否則會繼續扣款)
--   P0003:已經申請過了
CREATE FUNCTION request_tenant_deletion(p_grace_days int)
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

-- 取消刪除(寬限期內)。沒有在申請中回 false。
CREATE FUNCTION cancel_tenant_deletion()
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_tenant uuid := app_tenant_id();
    v_user   uuid := app_user_id();
BEGIN
    IF v_tenant IS NULL OR v_user IS NULL THEN
        RAISE EXCEPTION 'missing context' USING ERRCODE = '42501';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM memberships WHERE tenant_id = v_tenant AND user_id = v_user AND role = 'owner') THEN
        RAISE EXCEPTION 'owner only' USING ERRCODE = '42501';
    END IF;
    PERFORM 1 FROM tenants WHERE id = v_tenant FOR UPDATE;
    IF NOT EXISTS (SELECT 1 FROM tenants WHERE id = v_tenant AND deletion_scheduled_at IS NOT NULL) THEN
        RETURN false;
    END IF;
    UPDATE tenants SET deletion_requested_at = NULL, deletion_scheduled_at = NULL WHERE id = v_tenant;
    INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
    VALUES (v_tenant, 'user', v_user, 'tenant.deletion_cancelled', 'tenant', v_tenant, '{}');
    RETURN true;
END
$$;

-- ---------- 背景任務(只授權給 tenant_worker) ----------

-- 匿名化一位過期的顧客(與 routes/customers.rs 的 anonymize 同一套結果:姓名 / Email / 電話換掉、
-- 預約備註清掉、未寄出的信刪除、已寄出的信改成匿名地址),並寫一筆系統稽核。
-- 函式自己重新確認「真的過期、沒有進行中的預約、還沒匿名化」,不信任呼叫端挑出的名單。
-- customers 是 FORCE ROW LEVEL SECURITY,擁有者也受政策約束,所以先設定租戶上下文。
-- 回傳是否真的匿名化了。
CREATE FUNCTION retention_anonymize_customer(p_customer uuid, p_tenant uuid)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_email    text;
    v_days     int;
    v_created  timestamptz;
    v_last     timestamptz;
    v_anon     text;
    v_bookings int;
    v_unsent   int;
BEGIN
    PERFORM set_config('app.tenant_id', p_tenant::text, true);
    SELECT customer_retention_days INTO v_days FROM tenants WHERE id = p_tenant;
    IF v_days IS NULL THEN
        RETURN false;
    END IF;
    SELECT email::text, created_at INTO v_email, v_created
    FROM customers WHERE id = p_customer AND tenant_id = p_tenant FOR UPDATE;
    IF NOT FOUND OR v_email LIKE '%@anonymized.invalid' THEN
        RETURN false;
    END IF;
    IF EXISTS (SELECT 1 FROM bookings WHERE customer_id = p_customer
               AND (status = 'pending' OR (status = 'confirmed' AND starts_at > now()))) THEN
        RETURN false;
    END IF;
    SELECT max(starts_at) INTO v_last FROM bookings WHERE customer_id = p_customer;
    IF COALESCE(v_last, v_created) >= now() - make_interval(days => v_days) THEN
        RETURN false;
    END IF;

    v_anon := 'deleted-' || p_customer::text || '@anonymized.invalid';
    UPDATE customers SET name = '已刪除的顧客', email = v_anon, phone = NULL WHERE id = p_customer;
    UPDATE bookings SET notes = NULL WHERE customer_id = p_customer;
    GET DIAGNOSTICS v_bookings = ROW_COUNT;
    DELETE FROM email_outbox
    WHERE tenant_id = p_tenant AND status = 'pending' AND lower(to_email) = lower(v_email);
    GET DIAGNOSTICS v_unsent = ROW_COUNT;
    UPDATE email_outbox SET to_email = v_anon WHERE tenant_id = p_tenant AND lower(to_email) = lower(v_email);
    INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
    VALUES (p_tenant, 'system', NULL, 'customer.anonymized', 'customer', p_customer,
            jsonb_build_object('bookings', v_bookings, 'unsent_mails_removed', v_unsent, 'reason', 'retention'));
    RETURN true;
END
$$;

-- 清除超過 p_days 天的稽核日誌(一次最多 p_batch 筆),回傳刪了幾筆。
-- 稽核日誌對一般角色仍然只增不減;只有這個函式能刪,而且只刪「過期」的,下限 90 天。
-- 每個受影響的店家留一筆 audit.purged(筆數與截止時間),讓「為什麼看不到更早的紀錄」有交代。
CREATE FUNCTION retention_purge_audit(p_days int, p_batch int)
RETURNS int LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_cutoff timestamptz;
    v_total  int := 0;
    r        record;
BEGIN
    IF p_days IS NULL OR p_days < 90 THEN
        RAISE EXCEPTION 'retention too short' USING ERRCODE = '22023';
    END IF;
    v_cutoff := now() - make_interval(days => p_days);
    FOR r IN
        WITH doomed AS (
            SELECT id FROM audit_logs WHERE created_at < v_cutoff ORDER BY id LIMIT p_batch
        ), gone AS (
            DELETE FROM audit_logs a USING doomed d WHERE a.id = d.id RETURNING a.tenant_id
        )
        SELECT tenant_id, count(*)::int AS n FROM gone GROUP BY tenant_id
    LOOP
        v_total := v_total + r.n;
        INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
        VALUES (r.tenant_id, 'system', NULL, 'audit.purged', 'audit', NULL,
                jsonb_build_object('rows', r.n, 'older_than', v_cutoff));
    END LOOP;
    RETURN v_total;
END
$$;

-- 刪除寬限期已滿的店家(連同所有資料,由外鍵串聯刪除),回傳被刪除的店家 id。
-- FORCE ROW LEVEL SECURITY 的資料表(customers、services…)不受影響:外鍵的串聯動作不套用資料列安全政策
-- (已在受限帳號的叢集上驗證,見 tests/restricted_role.rs)。
CREATE FUNCTION retention_delete_tenants(p_batch int)
RETURNS SETOF uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    v_id uuid;
BEGIN
    FOR v_id IN
        SELECT id FROM tenants WHERE deletion_scheduled_at IS NOT NULL AND deletion_scheduled_at <= now()
        ORDER BY deletion_scheduled_at LIMIT p_batch FOR UPDATE SKIP LOCKED
    LOOP
        -- bookings 對成員 / 服務 / 顧客的外鍵沒有串聯刪除(為了保護有紀錄的資料),
        -- 串聯刪除店家時會卡在這裡,所以先明確刪掉預約
        DELETE FROM bookings WHERE tenant_id = v_id;
        DELETE FROM tenants WHERE id = v_id;
        RETURN NEXT v_id;
    END LOOP;
END
$$;

REVOKE ALL ON FUNCTION set_customer_retention(int), request_tenant_deletion(int), cancel_tenant_deletion() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION set_customer_retention(int), request_tenant_deletion(int), cancel_tenant_deletion() TO tenant_app;
REVOKE ALL ON FUNCTION retention_anonymize_customer(uuid, uuid), retention_purge_audit(int, int), retention_delete_tenants(int) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION retention_anonymize_customer(uuid, uuid), retention_purge_audit(int, int), retention_delete_tenants(int) TO tenant_worker;
