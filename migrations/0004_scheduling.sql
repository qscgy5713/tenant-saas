-- 員工可提供的服務
CREATE TABLE staff_services (
    tenant_id  uuid NOT NULL,
    user_id    uuid NOT NULL,
    service_id uuid NOT NULL,
    PRIMARY KEY (tenant_id, user_id, service_id),
    FOREIGN KEY (tenant_id, user_id)    REFERENCES memberships(tenant_id, user_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, service_id) REFERENCES services(tenant_id, id)         ON DELETE CASCADE
);

-- 每週營業時段:以店家本地時間儲存,計算可預約時段時再依 tenants.timezone 轉換。weekday: 0 = 週日
CREATE TABLE working_hours (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id  uuid NOT NULL,
    user_id    uuid NOT NULL,
    weekday    smallint NOT NULL CHECK (weekday BETWEEN 0 AND 6),
    start_time time NOT NULL,
    end_time   time NOT NULL,
    CHECK (end_time > start_time),
    FOREIGN KEY (tenant_id, user_id) REFERENCES memberships(tenant_id, user_id) ON DELETE CASCADE,
    -- 同一員工同一天的時段不可重疊
    EXCLUDE USING gist (
        tenant_id WITH =,
        user_id   WITH =,
        weekday   WITH =,
        tsrange(timestamp '2000-01-01' + start_time, timestamp '2000-01-01' + end_time) WITH &&
    )
);

CREATE TABLE time_off (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id  uuid NOT NULL,
    user_id    uuid NOT NULL,
    starts_at  timestamptz NOT NULL,
    ends_at    timestamptz NOT NULL,
    reason     text,
    CHECK (ends_at > starts_at),
    FOREIGN KEY (tenant_id, user_id) REFERENCES memberships(tenant_id, user_id) ON DELETE CASCADE
);
CREATE INDEX time_off_user_idx ON time_off (tenant_id, user_id, starts_at);

ALTER TABLE staff_services ENABLE ROW LEVEL SECURITY;
ALTER TABLE staff_services FORCE  ROW LEVEL SECURITY;
ALTER TABLE working_hours  ENABLE ROW LEVEL SECURITY;
ALTER TABLE working_hours  FORCE  ROW LEVEL SECURITY;
ALTER TABLE time_off       ENABLE ROW LEVEL SECURITY;
ALTER TABLE time_off       FORCE  ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation ON staff_services
    USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id());
CREATE POLICY tenant_isolation ON working_hours
    USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id());
CREATE POLICY tenant_isolation ON time_off
    USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id());

GRANT SELECT, INSERT, UPDATE, DELETE ON staff_services, working_hours, time_off TO tenant_app;
