-- 提醒信的去重鍵原本是 reminder:{booking_id},改期之後永遠不會再提醒。
-- 改成 reminder:{booking_id}:{reminder_seq},每次改期遞增 → 每個「排程輪次」各寄一封。
-- 不用開始時間當鍵:A → B → 再改回 A 時,鍵會與第一次相同而被去重吞掉。
ALTER TABLE bookings ADD COLUMN reminder_seq int NOT NULL DEFAULT 0;
