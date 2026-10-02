-- 預約確認流程:顧客自助預約先是 pending(不占時段,不在排除約束的條件內),
-- 對方按下信中的確認連結時才變 confirmed,屆時由排除約束把關。
-- 注意:新增的 enum 值在同一個 migration 交易內不能使用,所以這裡不引用 'pending'。
ALTER TYPE booking_status ADD VALUE IF NOT EXISTS 'pending';
ALTER TABLE bookings
    ADD COLUMN confirmed_at       timestamptz,
    ADD COLUMN reminder_queued_at timestamptz;
UPDATE bookings SET confirmed_at = created_at WHERE confirmed_at IS NULL;
CREATE INDEX bookings_starts_idx ON bookings (starts_at);

-- 寄信 outbox:和業務資料同一個交易寫入,不會出現「預約成功但信沒排進去」。
-- 內容含一次性連結,寄出(或最終失敗)後會清空 body,不長期保留 token。
CREATE TABLE email_outbox (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    to_email    text NOT NULL,
    subject     text NOT NULL,
    body        text NOT NULL,
    status      text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sent', 'failed')),
    attempts    int  NOT NULL DEFAULT 0,
    run_at      timestamptz NOT NULL DEFAULT now(),
    last_error  text,
    dedupe_key  text UNIQUE,                  -- 同一件事只會排進一封信
    created_at  timestamptz NOT NULL DEFAULT now(),
    sent_at     timestamptz
);
CREATE INDEX email_outbox_due_idx ON email_outbox (run_at) WHERE status = 'pending';

ALTER TABLE email_outbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE email_outbox FORCE  ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON email_outbox
    USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id());
-- 請求內只能排信、看自己店的信,不能改或刪
GRANT SELECT, INSERT ON email_outbox TO tenant_app;

-- 背景 worker 要跨租戶處理,但仍用獨立的最小權限角色(不是超級使用者、沒有 BYPASSRLS),
-- 只對它需要的資料表開放明確的政策。
DO $$
BEGIN
    CREATE ROLE tenant_worker NOLOGIN NOBYPASSRLS;
EXCEPTION WHEN duplicate_object OR unique_violation THEN
    NULL;
END $$;
GRANT tenant_worker TO CURRENT_USER;

GRANT SELECT, INSERT, UPDATE, DELETE ON email_outbox TO tenant_worker;
GRANT SELECT, UPDATE ON bookings TO tenant_worker;
GRANT SELECT ON customers, services, tenants TO tenant_worker;
GRANT SELECT (id, name) ON users TO tenant_worker;

CREATE POLICY worker_all ON email_outbox FOR ALL TO tenant_worker USING (true) WITH CHECK (true);
CREATE POLICY worker_all ON bookings     FOR ALL TO tenant_worker USING (true) WITH CHECK (true);
CREATE POLICY worker_read ON customers   FOR SELECT TO tenant_worker USING (true);
CREATE POLICY worker_read ON services    FOR SELECT TO tenant_worker USING (true);
CREATE POLICY worker_read ON tenants     FOR SELECT TO tenant_worker USING (true);
