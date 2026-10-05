-- 提醒信帶改期 / 取消連結。
--
-- 資料庫只存 token 的雜湊,所以提醒信沒辦法「重新取得」確認信裡的原始 token。
-- 解法:提醒信寄出時另外發一個專用的 token(同樣只存雜湊),和確認信的 token 並存,任何一個都能管理這筆預約。
-- 好處:確認信的連結不會因此失效;兩封信的連結是不同的祕密,外洩一個的影響範圍也只有那一筆預約。
-- 改期後會重新提醒、重新發一個,上一封提醒信的連結就失效(確認信的連結不受影響)。
ALTER TABLE bookings ADD COLUMN reminder_token_hash text UNIQUE;

-- 公開預約頁用 token 解析租戶:兩種 token 都認
CREATE OR REPLACE FUNCTION booking_tenant_by_token(p_token_hash text)
RETURNS uuid LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS
$$ SELECT b.tenant_id FROM bookings b JOIN tenants t ON t.id = b.tenant_id
   WHERE (b.manage_token_hash = p_token_hash OR b.reminder_token_hash = p_token_hash)
     AND t.status = 'active' $$;
