-- 提醒信的連結:改期後重新提醒時,舊的提醒信連結不再失效。
--
-- 原本 `reminder_token_hash` 只有一個,每次重新提醒就被覆蓋 → 顧客點舊的提醒信會看到「找不到」。
-- 但確認信的連結本來就一直有效,讓舊提醒連結失效並沒有換到任何安全性,只造成混亂。
-- 改成陣列,保留最近 10 個(每次改期後最多多一個,上限避免無限成長);任何一個都能管理這筆預約。
ALTER TABLE bookings ADD COLUMN reminder_token_hashes text[] NOT NULL DEFAULT '{}';
UPDATE bookings SET reminder_token_hashes = ARRAY[reminder_token_hash] WHERE reminder_token_hash IS NOT NULL;
CREATE INDEX bookings_reminder_tokens_idx ON bookings USING gin (reminder_token_hashes);

-- 公開預約頁用 token 解析租戶(其餘與 0028 相同)。要先換掉它,才能刪舊欄位
CREATE OR REPLACE FUNCTION booking_tenant_by_token(p_token_hash text)
RETURNS uuid LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS
$$ SELECT b.tenant_id FROM bookings b JOIN tenants t ON t.id = b.tenant_id
   WHERE (b.manage_token_hash = p_token_hash OR b.reminder_token_hashes @> ARRAY[p_token_hash])
     AND t.status = 'active' AND t.deletion_scheduled_at IS NULL $$;

ALTER TABLE bookings DROP COLUMN reminder_token_hash;
