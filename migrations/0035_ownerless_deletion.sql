-- CR 發現:店主可以在「申請刪除店家」之後刪除自己的帳號(店家會照期限刪掉,見 0030)。
-- 但到期時如果還有未來預約,0029 的安全網會**取消這次刪除** —— 這時店主已經不在了,
-- 店家會變成沒有任何在職擁有者:沒人能改設定、也沒人能再申請刪除。
--
-- 改成:還有未來預約時,如果店家**已經沒有在職的擁有者**,不取消刪除,而是先跳過、下一輪再試
-- (公開預約頁在申請刪除時就關閉了,不會再有新預約;既有的預約結束後就會照原本的決定刪除)。
-- 還有擁有者時維持 0029 的行為:取消刪除並留稽核,交給店主重新決定。
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
            IF EXISTS (SELECT 1 FROM memberships WHERE tenant_id = v_id AND role = 'owner' AND active) THEN
                UPDATE tenants SET deletion_requested_at = NULL, deletion_scheduled_at = NULL WHERE id = v_id;
                INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
                VALUES (v_id, 'system', NULL, 'tenant.deletion_cancelled', 'tenant', v_id,
                        jsonb_build_object('reason', 'live_bookings', 'count', v_live));
            END IF;
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
