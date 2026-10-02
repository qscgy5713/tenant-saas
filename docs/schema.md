# 資料表與 RLS 設計(待確認,尚未寫 migration)

## 設計原則
1. **租戶資料表一律有 `tenant_id`**,並啟用 RLS(`ENABLE` + `FORCE`),政策失敗時預設「看不到任何資料」。
2. **跨租戶參照用複合外鍵**:例如 `bookings(tenant_id, service_id)` 參照 `services(tenant_id, id)`,資料庫層就擋住「A 店的預約指向 B 店的服務」。
3. **兩個資料庫角色(M3 實作版)**:
   - 連線使用者(開發環境是 `tenant`):擁有資料表、執行 migration。擁有者與超級使用者預設會繞過 RLS。
   - `tenant_app`:`NOLOGIN NOBYPASSRLS`,只有必要的 SELECT / INSERT / UPDATE / DELETE。應用程式**不直接以它連線**,而是每個租戶交易開頭 `SET LOCAL ROLE tenant_app`(見 `src/db.rs` 的 `begin_scoped`),交易結束角色自動還原。
   - 正式環境部署時,連線使用者必須是 `tenant_app` 的成員(migration 只會 `GRANT tenant_app TO CURRENT_USER`,若 migrator 與應用程式連線帳號不同,需另外授權),且**不可是超級使用者**。
4. **租戶上下文**:`begin_scoped` 同一個交易內 `set_config('app.user_id' / 'app.tenant_id', ..., true)`(等同 `SET LOCAL`)。SQL 輔助函式 `app_user_id()`、`app_tenant_id()` 回傳 uuid,未設定時為 NULL,比對結果為 NULL,等於看不到任何資料。
5. 主鍵用 UUID(`gen_random_uuid()`),時間一律 `timestamptz`。

需要的 extension:`btree_gist`(排除約束)、`citext`(Email 不分大小寫)。

## 資料表總覽

| 表 | 層級 | RLS | 說明 |
|---|---|---|---|
| `users` | 全域 | 否 | 員工帳號,一個帳號可屬於多間店 |
| `tenants` | 全域 | 是(特殊) | 店家 |
| `memberships` | 全域 | 是(特殊) | 員工與店家的關係與角色 |
| `invitations` | 租戶 | 是 | 邀請員工加入 |
| `services` | 租戶 | 是 | 服務項目 |
| `staff_services` | 租戶 | 是 | 哪位員工可提供哪些服務 |
| `working_hours` | 租戶 | 是 | 員工每週營業時段 |
| `time_off` | 租戶 | 是 | 員工休假 / 不可預約時段 |
| `customers` | 租戶 | 是 | 顧客(不需註冊) |
| `bookings` | 租戶 | 是 | 預約 |
| `audit_logs` | 租戶 | 是 | 稽核日誌(只能新增) |

訂閱方案與限額(M7)、背景任務(M6)之後再加,不放進這一版。

## 全域表

```sql
CREATE TABLE users (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email         citext NOT NULL UNIQUE,
    password_hash text   NOT NULL,            -- argon2
    name          text   NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE tenants (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    slug       text NOT NULL UNIQUE
               CHECK (slug ~ '^[a-z0-9][a-z0-9-]{1,38}[a-z0-9]$'),  -- 子網域
    name       text NOT NULL,
    timezone   text NOT NULL DEFAULT 'Asia/Taipei',                 -- IANA 時區名
    status     text NOT NULL DEFAULT 'active' CHECK (status IN ('active','suspended')),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TYPE member_role AS ENUM ('owner', 'manager', 'staff');

CREATE TABLE memberships (
    tenant_id  uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id    uuid NOT NULL REFERENCES users(id)   ON DELETE CASCADE,
    role       member_role NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, user_id)
);
CREATE INDEX ON memberships (user_id);
```

### 全域表的 RLS(特殊)
`users` 不開 RLS(登入時要用 Email 查,此時還沒有租戶),但 `tenant_app` 只被授權讀 `id, email, name, created_at` 這幾個欄位,**讀不到 `password_hash`**(測試有驗證)。

`tenants`、`memberships` 要能支援「列出我所屬的所有店」,所以政策同時看使用者與租戶。以下是**實際 migration 的版本**(`0002_tenancy.sql`),以此為準:

```sql
-- memberships:已選定租戶時「只」看得到該租戶的成員;未選定時只看得到自己的關係。
-- (早期草稿用 OR,會讓使用者在 A 店的上下文裡同時看到自己在 B 店的關係,已修正。)
CREATE POLICY memberships_access ON memberships FOR ALL
  USING (CASE WHEN app_tenant_id() IS NOT NULL
              THEN tenant_id = app_tenant_id()
              ELSE user_id = app_user_id() END)
  WITH CHECK (tenant_id = app_tenant_id());

-- tenants:只讀。已選定租戶時只看得到它;未選定時看得到自己所屬的
CREATE POLICY tenants_select ON tenants FOR SELECT USING (
  id = app_tenant_id()
  OR (app_tenant_id() IS NULL
      AND id IN (SELECT tenant_id FROM memberships WHERE user_id = app_user_id()))
);
```

`tenants`、`memberships` 只 `ENABLE`、不 `FORCE` RLS:`SECURITY DEFINER` 函式以擁有者身分寫入,需要能繞過。租戶資料表(`services` 等)則 `ENABLE` + `FORCE`。

建立新店(INSERT tenants + 第一筆 owner membership)與「用 slug 解析租戶」這兩個動作,當下還沒有租戶上下文,用 `SECURITY DEFINER` 函式處理,只開放最小功能:

- `create_tenant(slug, name, timezone) -> uuid`:建立店家並把呼叫者設為 owner。**使用者取自請求上下文 `app_user_id()`,不接受參數**,避免替別人建立。已實作(M3)
- `tenant_id_by_slug(slug) -> uuid`:公開預約頁依子網域取得租戶 id(M4.5 實作)
- `accept_invitation(token_hash) -> uuid`(M5 實作,使用者取自上下文):被邀請者點連結時還不是成員,沒有租戶上下文,由函式驗證 token、檢查期限並建立 membership

已知限制:`users` 沒有 RLS,`tenant_app` 若遭 SQL injection 可讀到所有使用者的 Email 與密碼雜湊。緩解方式是應用程式只用參數化查詢,並在 M2 評估是否把 `users` 的查詢也包進 `SECURITY DEFINER` 函式(例如 `find_user_for_login(email)`),避免 `tenant_app` 直接擁有整張表的 SELECT。

## 租戶表

```sql
CREATE TABLE invitations (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    email       citext NOT NULL,
    role        member_role NOT NULL CHECK (role <> 'owner'),
    token_hash  text NOT NULL UNIQUE,         -- 只存雜湊,連結中的原始 token 不落地
    expires_at  timestamptz NOT NULL,
    accepted_at timestamptz,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE services (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name             text NOT NULL,
    duration_minutes int  NOT NULL CHECK (duration_minutes BETWEEN 5 AND 1440),
    price_cents      int  NOT NULL DEFAULT 0 CHECK (price_cents >= 0),
    active           boolean NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, id)                    -- 給複合外鍵參照
);

CREATE TABLE staff_services (
    tenant_id  uuid NOT NULL,
    user_id    uuid NOT NULL,
    service_id uuid NOT NULL,
    PRIMARY KEY (tenant_id, user_id, service_id),
    FOREIGN KEY (tenant_id, user_id)    REFERENCES memberships(tenant_id, user_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, service_id) REFERENCES services(tenant_id, id)         ON DELETE CASCADE
);

-- 每週營業時段:以「店家本地時間」儲存,計算時段時再依 tenants.timezone 轉成 UTC
CREATE TABLE working_hours (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id  uuid NOT NULL,
    user_id    uuid NOT NULL,
    weekday    smallint NOT NULL CHECK (weekday BETWEEN 0 AND 6),   -- 0 = 週日
    start_time time NOT NULL,
    end_time   time NOT NULL,
    CHECK (end_time > start_time),
    FOREIGN KEY (tenant_id, user_id) REFERENCES memberships(tenant_id, user_id) ON DELETE CASCADE
);

CREATE TABLE time_off (
    id        uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL,
    user_id   uuid NOT NULL,
    starts_at timestamptz NOT NULL,
    ends_at   timestamptz NOT NULL,
    reason    text,
    CHECK (ends_at > starts_at),
    FOREIGN KEY (tenant_id, user_id) REFERENCES memberships(tenant_id, user_id) ON DELETE CASCADE
);

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
```

## 預約(核心,已實作於 `0006_bookings.sql`)

與實作的差異:`manage_token_hash` 已實作;`bookings` 對員工 / 服務 / 顧客的外鍵**沒有** `ON DELETE CASCADE`,有預約紀錄的服務或成員無法直接刪除(API 回 409),避免靜默刪掉歷史資料。`tenant_app` 對 `bookings` 只有 SELECT / INSERT / UPDATE,沒有 DELETE。

```sql
CREATE TYPE booking_status AS ENUM ('confirmed', 'cancelled', 'completed', 'no_show');

CREATE TABLE bookings (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        uuid NOT NULL,
    staff_user_id    uuid NOT NULL,
    service_id       uuid NOT NULL,
    customer_id      uuid NOT NULL,
    starts_at        timestamptz NOT NULL,
    ends_at          timestamptz NOT NULL,
    status           booking_status NOT NULL DEFAULT 'confirmed',
    notes            text,
    manage_token_hash text NOT NULL UNIQUE,   -- 顧客取消 / 改期連結用,只存雜湊
    created_at       timestamptz NOT NULL DEFAULT now(),
    CHECK (ends_at > starts_at),
    FOREIGN KEY (tenant_id, staff_user_id) REFERENCES memberships(tenant_id, user_id),
    FOREIGN KEY (tenant_id, service_id)    REFERENCES services(tenant_id, id),
    FOREIGN KEY (tenant_id, customer_id)   REFERENCES customers(tenant_id, id),

    -- 同一位員工、同一時間只能有一筆「有效」預約;已取消的不佔時段
    EXCLUDE USING gist (
        tenant_id     WITH =,
        staff_user_id WITH =,
        tstzrange(starts_at, ends_at) WITH &&
    ) WHERE (status IN ('confirmed', 'completed', 'no_show'))
);
CREATE INDEX ON bookings (tenant_id, starts_at);
```

兩個人同時搶同一時段時,第二個 INSERT 會被資料庫以排除約束違規(SQLSTATE `23P01`)拒絕,應用層捕捉後回 409 即可,不需要自己加鎖。

## 稽核日誌

```sql
CREATE TABLE audit_logs (
    id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id     uuid NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    actor_user_id uuid REFERENCES users(id) ON DELETE SET NULL,  -- 顧客操作時為 NULL
    action        text NOT NULL,            -- 例如 booking.cancelled
    entity_type   text NOT NULL,
    entity_id     uuid,
    detail        jsonb NOT NULL DEFAULT '{}',
    created_at    timestamptz NOT NULL DEFAULT now()
);
-- tenant_app 只授予 SELECT, INSERT;不授予 UPDATE / DELETE
```

## 租戶表的 RLS 政策(統一模板)

每張租戶表(`invitations`、`services`、`staff_services`、`working_hours`、`time_off`、`customers`、`bookings`、`audit_logs`)套同一個模板:

```sql
ALTER TABLE services ENABLE ROW LEVEL SECURITY;
ALTER TABLE services FORCE  ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation ON services
  USING      (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid)
  WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid);
```

`nullif(..., '')` 是因為連線池重用連線時,`set_config(..., true)` 結束後設定值會變成空字串而不是 NULL,直接轉 uuid 會報錯。

## 權限(RBAC)對照

| 動作 | owner | manager | staff |
|---|---|---|---|
| 修改店家設定、刪除店家 | ✅ | ❌ | ❌ |
| 邀請 / 移除員工 | ✅ | ✅(不可動 owner) | ❌ |
| 管理服務項目 | ✅ | ✅ | ❌ |
| 管理所有員工的營業時間 | ✅ | ✅ | 只能管自己 |
| 查看所有預約 | ✅ | ✅ | 只看自己的 |
| 建立 / 取消預約 | ✅ | ✅ | 只能操作自己的 |
| 查看稽核日誌 | ✅ | ✅ | ❌ |

權限在應用層(axum extractor)檢查;RLS 只負責租戶隔離,不負責角色。

## 隔離測試清單(M3 必做)

狀態:1、2、3、4、6、7 已在 `tests/tenancy.rs` 實作並通過;5、8 要等 `bookings` 資料表(M4.5)。另做過破壞性驗證:移除 `services` 的 RLS 後有 5 項測試失敗,證明測試確實守著隔離。

1. 租戶 A 的請求查 `services`,看不到租戶 B 的資料
2. 未設定 `app.tenant_id` 時,所有租戶表查詢都回 0 筆
3. 租戶 A 嘗試 INSERT 帶 B 的 `tenant_id`,被 `WITH CHECK` 擋下
4. 租戶 A 嘗試 UPDATE / DELETE B 的資料,影響 0 筆
5. 複合外鍵:A 的預約指向 B 的服務,INSERT 失敗
6. 連線池重用:A 的請求結束後,同一條連線處理 B 的請求,不會殘留 A 的上下文
7. 應用角色 `tenant_app` 確實是 `NOBYPASSRLS`,且不是表擁有者
8. 併發:兩個請求同時預約同一時段,恰好一個成功、一個回 409

## 請你確認的決定
1. **員工即成員**:不另建 `staff` 表,員工就是 `memberships` 的使用者,只有登入的人才能被預約。(若要做「有員工但他們不登入」,需另建表。)
2. **一筆預約只有一位員工、一個服務**,暫不支援套餐或多人。
3. **顧客不需帳號**,以 Email 識別,用 `manage_token` 連結取消 / 改期。
4. **預約不設緩衝時間**(服務之間不留空檔),之後需要再加。
5. **平台超級管理員**(跨租戶後台)不在這一版,之後用獨立的資料庫角色與連線處理。
6. **主鍵用 `gen_random_uuid()`(UUIDv4)**,若在意索引寫入效率,之後可改成應用層產生 UUIDv7。

## 補充:M6 新增(`0007_jobs.sql`)
- `booking_status` 新增 `pending`(顧客自助預約、尚未按確認連結;**不在排除約束的條件內,不占時段**)
- `bookings.confirmed_at`(確認時間;提醒信規則用)、`bookings.reminder_queued_at`
- `email_outbox`:`dedupe_key` 唯一,同一件事只排一封信;`tenant_app` 只有 SELECT / INSERT(請求內只能排信,不能改或刪)
- 角色 `tenant_worker`:`NOLOGIN NOBYPASSRLS`,明確授權 `email_outbox`(完整)、`bookings`(讀 / 更新)、`customers`、`services`、`tenants`(唯讀)、`users(id, name)`;對應的 RLS 政策都寫明 `TO tenant_worker`
