-- 服務的整理時間(緩衝):這項服務結束後,這位員工多久不能接下一筆預約。
--
-- 顧客看到的預約開始 / 結束時間不變;只是員工的行程在 `blocked_until` 之前都算「被占住」。
--
-- `bookings.blocked_until` = 預約當下的 `ends_at + 該服務的整理時間`,是**快照**:
-- 之後店家改了服務的整理時間,不會讓已經存在的預約突然互相衝突,只影響新的預約。
-- 排除約束改成用 [starts_at, blocked_until),所以資料庫這道最後防線也會擋住「擠進整理時間」的並行請求。
ALTER TABLE services
    ADD COLUMN buffer_minutes int NOT NULL DEFAULT 0 CHECK (buffer_minutes BETWEEN 0 AND 120);

ALTER TABLE bookings ADD COLUMN blocked_until timestamptz;
UPDATE bookings SET blocked_until = ends_at;
ALTER TABLE bookings ALTER COLUMN blocked_until SET NOT NULL;
ALTER TABLE bookings ADD CONSTRAINT bookings_blocked_after_end CHECK (blocked_until >= ends_at);

-- 沒有指定 blocked_until 的寫入(手動修資料、直接 INSERT 的測試)預設等於 ends_at(沒有整理時間);
-- 只改 ends_at 的更新,整理時間的長度維持不變。應用程式一律明確寫入,這只是安全網。
CREATE FUNCTION bookings_set_blocked_until() RETURNS trigger LANGUAGE plpgsql AS
$$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.blocked_until := COALESCE(NEW.blocked_until, NEW.ends_at);
    ELSIF NEW.ends_at IS DISTINCT FROM OLD.ends_at
          AND NEW.blocked_until IS NOT DISTINCT FROM OLD.blocked_until THEN
        NEW.blocked_until := NEW.ends_at + (OLD.blocked_until - OLD.ends_at);
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER bookings_blocked_until BEFORE INSERT OR UPDATE ON bookings
    FOR EACH ROW EXECUTE FUNCTION bookings_set_blocked_until();

-- 換掉排除約束(原本的名稱是自動產生的,所以用查的)
DO $$
DECLARE
    c text;
BEGIN
    SELECT conname INTO c FROM pg_constraint
    WHERE conrelid = 'bookings'::regclass AND contype = 'x';
    EXECUTE format('ALTER TABLE bookings DROP CONSTRAINT %I', c);
END $$;
-- 同一位員工、同一時間只能有一筆「有效」預約(含整理時間);已取消 / 待確認的不占時段
ALTER TABLE bookings ADD CONSTRAINT bookings_no_overlap EXCLUDE USING gist (
    tenant_id     WITH =,
    staff_user_id WITH =,
    tstzrange(starts_at, blocked_until) WITH &&
) WHERE (status IN ('confirmed', 'completed', 'no_show'));
