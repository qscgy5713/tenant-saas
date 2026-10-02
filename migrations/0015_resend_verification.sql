-- 重寄確認信:顧客沒收到信就沒有 token,所以靠「預約編號 + Email」重寄,並換新的 token(舊連結失效)。
-- 次數與間隔限制記在預約上,避免被拿來對別人的信箱灌信。
ALTER TABLE bookings
    ADD COLUMN verification_resends int NOT NULL DEFAULT 0,
    ADD COLUMN verification_sent_at timestamptz NOT NULL DEFAULT now();
