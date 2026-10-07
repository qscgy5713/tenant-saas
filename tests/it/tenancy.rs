use crate::common::{add_member, app, call, create_tenant, signup};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tenant_saas::db::begin_scoped;
use uuid::Uuid;

// ---------- 資料庫層:RLS 隔離 ----------

struct Seed {
    user_a: Uuid,
    tenant_a: Uuid,
    user_b: Uuid,
    tenant_b: Uuid,
}

/// 以擁有者身分直接建立兩個互不相干的店家,各有一個 owner 和一個服務項目
async fn seed(pool: &PgPool) -> Seed {
    async fn one(pool: &PgPool, tag: &str) -> (Uuid, Uuid) {
        let user: Uuid = sqlx::query_scalar(
            "INSERT INTO users (email, password_hash, name) VALUES ($1, 'x', $2) RETURNING id",
        )
        .bind(format!("{tag}@example.com"))
        .bind(format!("user-{tag}"))
        .fetch_one(pool)
        .await
        .unwrap();
        let tenant: Uuid =
            sqlx::query_scalar("INSERT INTO tenants (slug, name) VALUES ($1, $2) RETURNING id")
                .bind(format!("shop-{tag}"))
                .bind(format!("Shop {tag}"))
                .fetch_one(pool)
                .await
                .unwrap();
        sqlx::query("INSERT INTO memberships (tenant_id, user_id, role) VALUES ($1, $2, 'owner')")
            .bind(tenant)
            .bind(user)
            .execute(pool)
            .await
            .unwrap();
        let mut tx = begin_scoped(pool, Some(user), Some(tenant)).await.unwrap();
        sqlx::query("INSERT INTO services (tenant_id, name, duration_minutes) VALUES ($1, $2, 30)")
            .bind(tenant)
            .bind(format!("service-{tag}"))
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        (user, tenant)
    }
    let (user_a, tenant_a) = one(pool, "a").await;
    let (user_b, tenant_b) = one(pool, "b").await;
    Seed {
        user_a,
        tenant_a,
        user_b,
        tenant_b,
    }
}

fn pg_code(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.to_string())
}

#[sqlx::test]
async fn tenant_only_sees_its_own_rows(pool: PgPool) {
    let s = seed(&pool).await;
    let mut tx = begin_scoped(&pool, Some(s.user_a), Some(s.tenant_a))
        .await
        .unwrap();
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM services ORDER BY name")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(names, vec!["service-a"]);
}

#[sqlx::test]
async fn no_context_sees_nothing(pool: PgPool) {
    seed(&pool).await;
    let mut tx = begin_scoped(&pool, None, None).await.unwrap();
    for (table, sql) in [
        ("services", "SELECT count(*) FROM services"),
        ("tenants", "SELECT count(*) FROM tenants"),
        ("memberships", "SELECT count(*) FROM memberships"),
    ] {
        let n: i64 = sqlx::query_scalar(sql).fetch_one(&mut *tx).await.unwrap();
        assert_eq!(n, 0, "{table} 在沒有租戶上下文時必須是 0 筆");
    }
}

#[sqlx::test]
async fn cannot_insert_rows_for_another_tenant(pool: PgPool) {
    let s = seed(&pool).await;
    let mut tx = begin_scoped(&pool, Some(s.user_a), Some(s.tenant_a))
        .await
        .unwrap();
    let err = sqlx::query(
        "INSERT INTO services (tenant_id, name, duration_minutes) VALUES ($1, 'evil', 30)",
    )
    .bind(s.tenant_b)
    .execute(&mut *tx)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&err).as_deref(),
        Some("42501"),
        "應被 WITH CHECK 擋下: {err}"
    );
}

#[sqlx::test]
async fn cannot_update_or_delete_another_tenants_rows(pool: PgPool) {
    let s = seed(&pool).await;
    let mut tx = begin_scoped(&pool, Some(s.user_a), Some(s.tenant_a))
        .await
        .unwrap();
    let updated = sqlx::query("UPDATE services SET name = 'hacked' WHERE tenant_id = $1")
        .bind(s.tenant_b)
        .execute(&mut *tx)
        .await
        .unwrap()
        .rows_affected();
    let deleted = sqlx::query("DELETE FROM services WHERE tenant_id = $1")
        .bind(s.tenant_b)
        .execute(&mut *tx)
        .await
        .unwrap()
        .rows_affected();
    assert_eq!((updated, deleted), (0, 0));
    tx.commit().await.unwrap();

    let mut tx = begin_scoped(&pool, Some(s.user_b), Some(s.tenant_b))
        .await
        .unwrap();
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM services")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(names, vec!["service-b"]);
}

#[sqlx::test]
async fn context_does_not_leak_through_connection_reuse(pool: PgPool) {
    let s = seed(&pool).await;
    // 只允許 1 條連線,強迫前後兩個請求共用同一條實體連線
    let single = PgPoolOptions::new()
        .max_connections(1)
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();

    let mut tx = begin_scoped(&single, Some(s.user_a), Some(s.tenant_a))
        .await
        .unwrap();
    sqlx::query("SELECT 1").execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();

    // 交易結束後:角色還原、上下文清空
    let (role, tenant, user): (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT current_user::text, current_setting('app.tenant_id', true), current_setting('app.user_id', true)",
    )
    .fetch_one(&single)
    .await
    .unwrap();
    assert_ne!(role, "tenant_app");
    assert!(tenant.unwrap_or_default().is_empty());
    assert!(user.unwrap_or_default().is_empty());

    // 沒有 commit 直接放棄(rollback)的交易也不能殘留
    let tx = begin_scoped(&single, Some(s.user_a), Some(s.tenant_a))
        .await
        .unwrap();
    drop(tx);
    let mut tx = begin_scoped(&single, Some(s.user_b), Some(s.tenant_b))
        .await
        .unwrap();
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM services")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(names, vec!["service-b"]);
}

#[sqlx::test]
async fn app_role_is_unprivileged(pool: PgPool) {
    let s = seed(&pool).await;
    let mut tx = begin_scoped(&pool, Some(s.user_a), Some(s.tenant_a))
        .await
        .unwrap();

    let (role, bypass, superuser): (String, bool, bool) = sqlx::query_as(
        "SELECT current_user::text, rolbypassrls, rolsuper FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(role, "tenant_app");
    assert!(!bypass && !superuser);

    let owners: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT tableowner::text FROM pg_tables WHERE schemaname = 'public'",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    assert!(
        !owners.contains(&"tenant_app".to_string()),
        "應用角色不能是資料表擁有者"
    );

    // 密碼雜湊欄位沒有授權
    let err = sqlx::query("SELECT password_hash FROM users")
        .fetch_all(&mut *tx)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&err).as_deref(), Some("42501"));
}

#[sqlx::test]
async fn memberships_are_scoped_by_context(pool: PgPool) {
    let s = seed(&pool).await;
    // user_a 同時也是 tenant_b 的 staff
    sqlx::query("INSERT INTO memberships (tenant_id, user_id, role) VALUES ($1, $2, 'staff')")
        .bind(s.tenant_b)
        .bind(s.user_a)
        .execute(&pool)
        .await
        .unwrap();

    // 沒選租戶:只看得到自己的(2 筆),看不到 user_b 的 owner 關係
    let mut tx = begin_scoped(&pool, Some(s.user_a), None).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM memberships")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(n, 2);
    drop(tx);

    // 選了 tenant_a:只看得到 tenant_a 的成員,即使 user_a 在 tenant_b 也有關係
    let mut tx = begin_scoped(&pool, Some(s.user_a), Some(s.tenant_a))
        .await
        .unwrap();
    let tenants: Vec<Uuid> = sqlx::query_scalar("SELECT DISTINCT tenant_id FROM memberships")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(tenants, vec![s.tenant_a]);
}

// ---------- API 層 ----------

#[sqlx::test]
async fn create_and_list_tenants(pool: PgPool) {
    let app = app(pool);
    let token = signup(&app, "a@example.com").await;

    let (status, body) = create_tenant(&app, &token, "shop-a").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["role"], "owner");
    assert_eq!(body["timezone"], "Asia/Taipei");

    let (status, list) = call(&app, Method::GET, "/tenants", None, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);

    let (status, me) = call(&app, Method::GET, "/t/shop-a/me", None, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["role"], "owner");
}

#[sqlx::test]
async fn create_tenant_validation(pool: PgPool) {
    let app = app(pool);
    let token = signup(&app, "a@example.com").await;

    assert_eq!(
        create_tenant(&app, &token, "Bad Slug").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        create_tenant(&app, &token, "-abc").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        create_tenant(&app, &token, "ab").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        create_tenant(&app, &token, "admin").await.0,
        StatusCode::BAD_REQUEST
    );

    let (status, _) = call(
        &app,
        Method::POST,
        "/tenants",
        Some(json!({"slug": "shop-a", "name": "x", "timezone": "Mars/Base"})),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    assert_eq!(
        create_tenant(&app, &token, "shop-a").await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        create_tenant(&app, &token, "shop-a").await.0,
        StatusCode::CONFLICT
    );

    let (status, _) = call(
        &app,
        Method::POST,
        "/tenants",
        Some(json!({"slug": "shop-z", "name": "x"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn non_member_gets_404_same_as_missing_tenant(pool: PgPool) {
    let app = app(pool);
    let owner = signup(&app, "a@example.com").await;
    let stranger = signup(&app, "b@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;

    let not_member = call(
        &app,
        Method::GET,
        "/t/shop-a/members",
        None,
        Some(&stranger),
    )
    .await;
    let missing = call(
        &app,
        Method::GET,
        "/t/nope-nope/members",
        None,
        Some(&stranger),
    )
    .await;
    assert_eq!(not_member.0, StatusCode::NOT_FOUND);
    assert_eq!(not_member, missing, "不可洩漏店家是否存在");

    let (status, _) = call(&app, Method::GET, "/t/shop-a/members", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn one_user_can_belong_to_several_tenants(pool: PgPool) {
    let app = app(pool);
    let u1 = signup(&app, "a@example.com").await;
    let u2 = signup(&app, "b@example.com").await;
    create_tenant(&app, &u1, "shop-a").await;
    create_tenant(&app, &u1, "shop-b").await;
    create_tenant(&app, &u2, "shop-c").await;

    let (_, list) = call(&app, Method::GET, "/tenants", None, Some(&u1)).await;
    let slugs: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs.len(), 2);
    assert!(slugs.contains(&"shop-a") && slugs.contains(&"shop-b"));

    // 各店只看得到自己的成員
    for slug in ["shop-a", "shop-b"] {
        let (status, members) = call(
            &app,
            Method::GET,
            &format!("/t/{slug}/members"),
            None,
            Some(&u1),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let members = members.as_array().unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0]["email"], "a@example.com");
        assert!(members[0].get("password_hash").is_none());
    }
    // u2 看不到 u1 的店
    assert_eq!(
        call(&app, Method::GET, "/t/shop-a/me", None, Some(&u2))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

// ---------- 修改店家名稱 / 時區 ----------

#[sqlx::test]
async fn only_the_owner_can_rename_the_shop_or_change_its_timezone(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "owner@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    let manager = signup(&app, "m@example.com").await;
    add_member(&pool, "shop-a", "m@example.com", "manager").await;

    let patch = |token: &str, body: Value| {
        let (app, token) = (app.clone(), token.to_string());
        async move { call(&app, Method::PATCH, "/t/shop-a", Some(body), Some(&token)).await }
    };

    let (status, body) = patch(
        &owner,
        json!({"name": "  新店名  ", "timezone": "Asia/Tokyo"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (
            body["name"].as_str(),
            body["timezone"].as_str(),
            body["slug"].as_str()
        ),
        (Some("新店名"), Some("Asia/Tokyo"), Some("shop-a"))
    );

    // 只改其中一個欄位,另一個保持
    let (_, body) = patch(&owner, json!({"name": "再改一次"})).await;
    assert_eq!(
        (body["name"].as_str(), body["timezone"].as_str()),
        (Some("再改一次"), Some("Asia/Tokyo"))
    );

    // 管理者不行,資料不變
    assert_eq!(
        patch(&manager, json!({"name": "駭客"})).await.0,
        StatusCode::FORBIDDEN
    );
    let name: String = sqlx::query_scalar("SELECT name FROM tenants WHERE slug = 'shop-a'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "再改一次");

    // 驗證
    for bad in [
        json!({}),
        json!({"name": "   "}),
        json!({"name": "x".repeat(101)}),
        json!({"timezone": "Mars/Base"}),
        json!({"timezone": ""}),
    ] {
        assert_eq!(
            patch(&owner, bad.clone()).await.0,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    // 網址代稱與方案不能經由這個端點改(多餘的欄位被忽略)
    patch(
        &owner,
        json!({"name": "店", "slug": "hacked", "plan_id": "business"}),
    )
    .await;
    let (slug, plan): (String, String) = sqlx::query_as("SELECT slug, plan_id FROM tenants")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((slug.as_str(), plan.as_str()), ("shop-a", "free"));

    // 未登入
    assert_eq!(
        call(
            &app,
            Method::PATCH,
            "/t/shop-a",
            Some(json!({"name": "x"})),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test]
async fn changing_the_timezone_is_audited_and_visible_to_customers(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "owner@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    call(
        &app,
        Method::PATCH,
        "/t/shop-a",
        Some(json!({"timezone": "America/New_York"})),
        Some(&owner),
    )
    .await;
    call(
        &app,
        Method::PATCH,
        "/t/shop-a",
        Some(json!({"name": "只改名"})),
        Some(&owner),
    )
    .await;

    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT detail FROM audit_logs WHERE action = 'tenant.updated' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (
            rows[0]["timezone_from"].as_str(),
            rows[0]["timezone_to"].as_str()
        ),
        (Some("Asia/Taipei"), Some("America/New_York"))
    );
    assert_eq!(rows[0]["name_changed"], json!(false), "第一次沒改名");
    assert_eq!(rows[1]["name_changed"], json!(true));
    assert!(
        rows[1].get("timezone_to").is_none(),
        "沒改時區就不記錄時區: {}",
        rows[1]
    );

    let (_, shop) = call(&app, Method::GET, "/public/shops/shop-a", None, None).await;
    assert_eq!(shop["timezone"], "America/New_York");
}
