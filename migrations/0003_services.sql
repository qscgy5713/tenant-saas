-- 代表性的租戶資料表。CRUD 在 M4 實作,這裡先建好以驗證 RLS 隔離。
CREATE TABLE services (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name             text NOT NULL,
    duration_minutes int  NOT NULL CHECK (duration_minutes BETWEEN 5 AND 1440),
    price_cents      int  NOT NULL DEFAULT 0 CHECK (price_cents >= 0),
    active           boolean NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id)
);

ALTER TABLE services ENABLE ROW LEVEL SECURITY;
ALTER TABLE services FORCE  ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation ON services
    USING      (tenant_id = app_tenant_id())
    WITH CHECK (tenant_id = app_tenant_id());

GRANT SELECT, INSERT, UPDATE, DELETE ON services TO tenant_app;
