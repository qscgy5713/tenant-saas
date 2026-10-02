CREATE TABLE customers (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id  uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name       text NOT NULL,
    email      citext NOT NULL,
    phone      text,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, email),
    UNIQUE (tenant_id, id)
);

CREATE TYPE booking_status AS ENUM ('confirmed', 'cancelled', 'completed', 'no_show');

CREATE TABLE bookings (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id         uuid NOT NULL,
    staff_user_id     uuid NOT NULL,
    service_id        uuid NOT NULL,
    customer_id       uuid NOT NULL,
    starts_at         timestamptz NOT NULL,
    ends_at           timestamptz NOT NULL,
    status            booking_status NOT NULL DEFAULT 'confirmed',
    notes             text,
    manage_token_hash text NOT NULL UNIQUE,   -- 顧客取消 / 改期連結用,只存雜湊
    created_at        timestamptz NOT NULL DEFAULT now(),
    CHECK (ends_at > starts_at),
    -- 沒有 ON DELETE CASCADE:有預約紀錄的員工 / 服務 / 顧客不能被直接刪除
    FOREIGN KEY (tenant_id, staff_user_id) REFERENCES memberships(tenant_id, user_id),
    FOREIGN KEY (tenant_id, service_id)    REFERENCES services(tenant_id, id),
    FOREIGN KEY (tenant_id, customer_id)   REFERENCES customers(tenant_id, id),

    -- 同一位員工、同一時間只能有一筆「有效」預約;已取消的不佔時段。
    -- 這是防止重複預約的最後一道防線:兩個請求同時搶同一時段,資料庫只放行一個。
    EXCLUDE USING gist (
        tenant_id     WITH =,
        staff_user_id WITH =,
        tstzrange(starts_at, ends_at) WITH &&
    ) WHERE (status IN ('confirmed', 'completed', 'no_show'))
);
CREATE INDEX bookings_tenant_start_idx ON bookings (tenant_id, starts_at);
CREATE INDEX bookings_customer_idx ON bookings (tenant_id, customer_id, starts_at);

ALTER TABLE customers ENABLE ROW LEVEL SECURITY;
ALTER TABLE customers FORCE  ROW LEVEL SECURITY;
ALTER TABLE bookings  ENABLE ROW LEVEL SECURITY;
ALTER TABLE bookings  FORCE  ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation ON customers
    USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id());
CREATE POLICY tenant_isolation ON bookings
    USING (tenant_id = app_tenant_id()) WITH CHECK (tenant_id = app_tenant_id());

-- 預約不能被刪除,只能改狀態
GRANT SELECT, INSERT, UPDATE ON customers, bookings TO tenant_app;

-- 公開預約頁:請求只帶店家網域代稱或預約連結 token,尚無租戶上下文,
-- 用最小功能的 SECURITY DEFINER 函式解析出租戶 id,之後照一般流程設定上下文並受 RLS 約束。
CREATE FUNCTION tenant_id_by_slug(p_slug text)
RETURNS uuid LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS
$$ SELECT id FROM tenants WHERE slug = p_slug AND status = 'active' $$;

CREATE FUNCTION booking_tenant_by_token(p_token_hash text)
RETURNS uuid LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS
$$ SELECT b.tenant_id FROM bookings b JOIN tenants t ON t.id = b.tenant_id
   WHERE b.manage_token_hash = p_token_hash AND t.status = 'active' $$;

REVOKE ALL ON FUNCTION tenant_id_by_slug(text), booking_tenant_by_token(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION tenant_id_by_slug(text), booking_tenant_by_token(text) TO tenant_app;
