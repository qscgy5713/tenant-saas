//! 資料保留:顧客個資到期自動匿名化、稽核日誌到期清除、刪除店家(30 天寬限)。
mod common;

use axum::{
    Router,
    http::{Method, StatusCode},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Asia::Taipei;
use common::{add_member, app, call, create_service, create_tenant, signup, user_id};
use serde_json::{Value, json};
use sqlx::PgPool;
use tenant_saas::{availability::local_to_utc, worker::retention_sweep};
use uuid::Uuid;

fn day(offset: i64) -> NaiveDate {
    Utc::now().with_timezone(&Taipei).date_naive() + Duration::days(offset)
}

fn at(date: NaiveDate, h: u32) -> DateTime<Utc> {
    local_to_utc(
        Taipei,
        date,
        chrono::NaiveTime::from_hms_opt(h, 0, 0).unwrap(),
    )
    .unwrap()
}

struct Shop {
    app: Router,
    owner: String,
    owner_id: Uuid,
    service_id: String,
}

/// 開店、一個 30 分鐘的服務,店主每天 09:00–18:00 提供
async fn shop(pool: &PgPool, slug: &str, email: &str) -> Shop {
    let app = app(pool.clone());
    let owner = signup(&app, email).await;
    assert_eq!(
        create_tenant(&app, &owner, slug).await.0,
        StatusCode::CREATED
    );
    let service = create_service(&app, &owner, slug, "剪髮").await;
    let service_id = service["id"].as_str().unwrap().to_string();
    let owner_id = user_id(pool, email).await;
    let hours: Vec<Value> = (0..7)
        .map(|d| json!({"weekday": d, "start": "09:00", "end": "18:00"}))
        .collect();
    for (path, body) in [
        (
            format!("/t/{slug}/members/{owner_id}/working-hours"),
            json!({"hours": hours}),
        ),
        (
            format!("/t/{slug}/members/{owner_id}/services"),
            json!({"service_ids": [service_id]}),
        ),
    ] {
        let (s, b) = call(&app, Method::PUT, &path, Some(body), Some(&owner)).await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }
    Shop {
        app,
        owner,
        owner_id,
        service_id,
    }
}

async fn book(s: &Shop, slug: &str, start: DateTime<Utc>, email: &str) -> (StatusCode, Value) {
    call(
        &s.app,
        Method::POST,
        &format!("/t/{slug}/bookings"),
        Some(json!({
            "service_id": s.service_id,
            "staff_id": s.owner_id,
            "start": start.to_rfc3339(),
            "customer": {"name": "王小明", "email": email, "phone": "0912345678"},
        })),
        Some(&s.owner),
    )
    .await
}

/// 預約一筆再把它挪到 `days_ago` 天前並標成已完成,同時寫上備註(含個資)
async fn past_booking(
    pool: &PgPool,
    s: &Shop,
    slug: &str,
    email: &str,
    days_ago: i64,
    slot: i64,
) -> (Uuid, Uuid) {
    let (status, body) = book(s, slug, at(day(3 + slot), 10), email).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    sqlx::query(
        "UPDATE bookings SET starts_at = now() - make_interval(days => $2::int),
                ends_at = now() - make_interval(days => $2::int) + interval '30 minutes',
                blocked_until = now() - make_interval(days => $2::int) + interval '30 minutes',
                status = 'completed', notes = '對花生過敏,電話 0912345678'
         WHERE id = $1",
    )
    .bind(id)
    .bind(days_ago as i32)
    .execute(pool)
    .await
    .unwrap();
    let customer: Uuid = sqlx::query_scalar("SELECT customer_id FROM bookings WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
    (id, customer)
}

async fn customer(pool: &PgPool, id: Uuid) -> (String, String, Option<String>) {
    sqlx::query_as("SELECT name, email::text, phone FROM customers WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn is_anonymized(c: &(String, String, Option<String>)) -> bool {
    c.0 == "已刪除的顧客" && c.1.ends_with("@anonymized.invalid") && c.2.is_none()
}

async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

// ---------- 顧客個資到期匿名化 ----------

#[sqlx::test]
async fn customers_are_anonymized_after_the_retention_period_and_not_before(pool: PgPool) {
    let s = shop(&pool, "shop-a", "a@example.com").await;
    let (old_booking, old) = past_booking(&pool, &s, "shop-a", "old@example.com", 800, 0).await;
    let (_, recent) = past_booking(&pool, &s, "shop-a", "recent@example.com", 100, 1).await;
    // 最近的預約在保留期內 → 即使顧客很早就建立,也不能匿名化
    let (_, mixed) = past_booking(&pool, &s, "shop-a", "mixed@example.com", 900, 2).await;
    let (status, _) = book(&s, "shop-a", at(day(5), 11), "mixed@example.com").await;
    assert_eq!(status, StatusCode::CREATED);
    // 沒有預約、很久以前建立的顧客:以建立時間起算
    let lonely: Uuid = sqlx::query_scalar(
        "INSERT INTO customers (tenant_id, name, email, created_at)
         SELECT id, '孤單', 'lonely@example.com', now() - interval '800 days' FROM tenants RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    // 還沒寄出的信:匿名化時要刪掉
    sqlx::query(
        "INSERT INTO email_outbox (tenant_id, to_email, subject, body)
         SELECT id, 'old@example.com', '提醒', '您好' FROM tenants",
    )
    .execute(&pool)
    .await
    .unwrap();

    let stats = retention_sweep(&pool, Some(730)).await;
    assert_eq!(stats.customers_anonymized, 2);

    for id in [old, lonely] {
        assert!(is_anonymized(&customer(&pool, id).await), "{id}");
    }
    for id in [recent, mixed] {
        assert!(!is_anonymized(&customer(&pool, id).await), "{id}");
    }
    // 預約本身保留,只清掉備註(備註常常寫著個資)
    let notes: Option<String> = sqlx::query_scalar("SELECT notes FROM bookings WHERE id = $1")
        .bind(old_booking)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(notes, None);
    assert_eq!(count(&pool, "SELECT count(*) FROM bookings").await, 4);
    // 沒寄出的信刪了
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM email_outbox WHERE status = 'pending' AND to_email = 'old@example.com'"
        )
        .await,
        0
    );
    // 稽核:系統做的、說明是保留政策、不含個資
    let rows: Vec<(String, Option<Uuid>, Value)> = sqlx::query_as(
        "SELECT actor_type, actor_user_id, detail FROM audit_logs WHERE action = 'customer.anonymized'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    for (actor, user, detail) in &rows {
        assert_eq!((actor.as_str(), user), ("system", &None));
        assert_eq!(detail["reason"], "retention");
        assert!(!detail.to_string().contains("example.com"));
    }

    // 再跑一次不會重複處理
    assert_eq!(
        retention_sweep(&pool, Some(730)).await.customers_anonymized,
        0
    );
}

#[sqlx::test]
async fn each_shop_can_shorten_its_own_retention_and_only_the_owner_may(pool: PgPool) {
    let s = shop(&pool, "shop-a", "a@example.com").await;
    let other = shop(&pool, "shop-b", "b@example.com").await;
    let (_, mine) = past_booking(&pool, &s, "shop-a", "mine@example.com", 100, 0).await;
    let (_, theirs) = past_booking(&pool, &other, "shop-b", "theirs@example.com", 100, 0).await;

    // 預設 730 天:100 天前的不動
    assert_eq!(
        retention_sweep(&pool, Some(730)).await.customers_anonymized,
        0
    );

    let put = |token: &str, days: i64| {
        let (app, token) = (s.app.clone(), token.to_string());
        async move {
            call(
                &app,
                Method::PUT,
                "/t/shop-a/data-retention",
                Some(json!({"customer_retention_days": days})),
                Some(&token),
            )
            .await
        }
    };
    // 範圍:90–3650
    assert_eq!(put(&s.owner, 89).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(put(&s.owner, 3651).await.0, StatusCode::BAD_REQUEST);
    // 只有店主
    let manager = signup(&s.app, "m@example.com").await;
    add_member(&pool, "shop-a", "m@example.com", "manager").await;
    assert_eq!(put(&manager, 90).await.0, StatusCode::FORBIDDEN);

    let (status, body) = put(&s.owner, 90).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["customer_retention_days"], 90);
    assert_eq!(
        retention_sweep(&pool, Some(730)).await.customers_anonymized,
        1
    );
    assert!(is_anonymized(&customer(&pool, mine).await));
    // 別家店不受影響
    assert!(!is_anonymized(&customer(&pool, theirs).await));
    let audit: Value = sqlx::query_scalar(
        "SELECT detail FROM audit_logs WHERE action = 'tenant.retention_changed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit, json!({"from": 730, "to": 90}));
}

#[sqlx::test]
async fn the_database_function_rechecks_instead_of_trusting_the_candidate_list(pool: PgPool) {
    let s = shop(&pool, "shop-a", "a@example.com").await;
    let (_, young) = past_booking(&pool, &s, "shop-a", "young@example.com", 10, 0).await;
    let (_, live) = past_booking(&pool, &s, "shop-a", "live@example.com", 900, 1).await;
    book(&s, "shop-a", at(day(6), 10), "live@example.com").await;
    let tenant: Uuid = sqlx::query_scalar("SELECT id FROM tenants")
        .fetch_one(&pool)
        .await
        .unwrap();
    // 很久以前申請、一直卡在「待確認」的預約(例如 worker 長時間沒跑):顧客仍算有進行中的預約
    let (stuck_booking, stuck) =
        past_booking(&pool, &s, "shop-a", "stuck@example.com", 950, 2).await;
    sqlx::query("UPDATE bookings SET status = 'pending' WHERE id = $1")
        .bind(stuck_booking)
        .execute(&pool)
        .await
        .unwrap();
    for id in [young, live, stuck] {
        let did: bool = sqlx::query_scalar("SELECT retention_anonymize_customer($1, $2)")
            .bind(id)
            .bind(tenant)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!did, "{id} 不該被匿名化");
        assert!(!is_anonymized(&customer(&pool, id).await));
    }
}

#[sqlx::test]
async fn one_bad_customer_does_not_block_the_others(pool: PgPool) {
    let s = shop(&pool, "shop-a", "a@example.com").await;
    let (_, poison) = past_booking(&pool, &s, "shop-a", "poison@example.com", 800, 0).await;
    let (_, fine) = past_booking(&pool, &s, "shop-a", "fine@example.com", 850, 1).await;
    // 讓「poison」那位顧客的更新一定出錯(模擬資料有問題)
    sqlx::query(
        "CREATE FUNCTION boom() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'boom'; END $$",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER boom BEFORE UPDATE ON customers FOR EACH ROW
         WHEN (OLD.email = 'poison@example.com') EXECUTE FUNCTION boom()",
    )
    .execute(&pool)
    .await
    .unwrap();

    let stats = retention_sweep(&pool, Some(730)).await;
    assert_eq!(stats.customers_anonymized, 1, "出錯的那位不能拖累其他人");
    assert!(is_anonymized(&customer(&pool, fine).await));
    assert!(!is_anonymized(&customer(&pool, poison).await));
    // 出錯的那位沒有留下任何半套的稽核
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM audit_logs WHERE action = 'customer.anonymized'"
        )
        .await,
        1
    );
}

// ---------- 稽核日誌到期清除 ----------

#[sqlx::test]
async fn expired_audit_rows_are_purged_but_recent_ones_and_a_receipt_remain(pool: PgPool) {
    let s = shop(&pool, "shop-a", "a@example.com").await;
    let before = count(&pool, "SELECT count(*) FROM audit_logs").await;
    assert!(before > 0);
    // 把其中 3 筆挪到 800 天前
    sqlx::query(
        "UPDATE audit_logs SET created_at = now() - interval '800 days'
         WHERE id IN (SELECT id FROM audit_logs ORDER BY id LIMIT 3)",
    )
    .execute(&pool)
    .await
    .unwrap();

    // 沒設定保留天數(永久保留):什麼都不刪
    assert_eq!(retention_sweep(&pool, None).await.audit_rows_purged, 0);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM audit_logs").await,
        before
    );

    assert_eq!(retention_sweep(&pool, Some(730)).await.audit_rows_purged, 3);
    // 剩下的是近期的 + 一筆「清除了幾筆」的收據
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM audit_logs WHERE created_at < now() - interval '700 days'"
        )
        .await,
        0
    );
    let receipt: Value =
        sqlx::query_scalar("SELECT detail FROM audit_logs WHERE action = 'audit.purged'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(receipt["rows"], 3);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM audit_logs").await,
        before - 3 + 1
    );
    // 一般使用者仍然看得到收據(後台稽核頁)
    let (status, body) = call(
        &s.app,
        Method::GET,
        "/t/shop-a/audit-logs?action=audit.purged",
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[sqlx::test]
async fn audit_purge_refuses_a_retention_below_ninety_days(pool: PgPool) {
    shop(&pool, "shop-a", "a@example.com").await;
    let err = sqlx::query("SELECT retention_purge_audit(30, 100)")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("22023")
    );
    let err = sqlx::query("SELECT retention_purge_audit(NULL, 100)")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("22023")
    );
}

// ---------- 刪除店家 ----------

async fn request_delete(s: &Shop, slug: &str, token: &str, confirm: &str) -> (StatusCode, Value) {
    call(
        &s.app,
        Method::POST,
        &format!("/t/{slug}/deletion"),
        Some(json!({"confirm_slug": confirm})),
        Some(token),
    )
    .await
}

async fn cancel_delete(s: &Shop, slug: &str) -> (StatusCode, Value) {
    call(
        &s.app,
        Method::DELETE,
        &format!("/t/{slug}/deletion"),
        None,
        Some(&s.owner),
    )
    .await
}

#[sqlx::test]
async fn deleting_a_shop_has_a_grace_period_and_can_be_cancelled(pool: PgPool) {
    let s = shop(&pool, "shop-a", "a@example.com").await;
    let public = |app: &Router| {
        let app = app.clone();
        async move {
            call(&app, Method::GET, "/public/shops/shop-a", None, None)
                .await
                .0
        }
    };
    assert_eq!(public(&s.app).await, StatusCode::OK);

    // 還有未來的預約:不能申請
    let (status, booking) = book(&s, "shop-a", at(day(3), 10), "c@example.com").await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = request_delete(&s, "shop-a", &s.owner, "shop-a").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("預約"));
    let id = booking["id"].as_str().unwrap();
    let manage = booking["manage_token"].as_str().unwrap().to_string();
    let manage_status = |app: &Router| {
        let (app, uri) = (app.clone(), format!("/public/bookings/{manage}"));
        async move { call(&app, Method::GET, &uri, None, None).await.0 }
    };
    assert_eq!(manage_status(&s.app).await, StatusCode::OK);
    let (status, _) = call(
        &s.app,
        Method::PATCH,
        &format!("/t/shop-a/bookings/{id}"),
        Some(json!({"status": "cancelled"})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 只有店主;網址代稱要打對
    let manager = signup(&s.app, "m@example.com").await;
    add_member(&pool, "shop-a", "m@example.com", "manager").await;
    assert_eq!(
        request_delete(&s, "shop-a", &manager, "shop-a").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request_delete(&s, "shop-a", &s.owner, "shop-b").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM tenants WHERE deletion_scheduled_at IS NOT NULL"
        )
        .await,
        0
    );

    // 申請成功:約 30 天後
    let (status, body) = request_delete(&s, "shop-a", &s.owner, "shop-a").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let when: DateTime<Utc> = body["deletion_scheduled_at"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let days = (when - Utc::now()).num_hours() as f64 / 24.0;
    assert!((29.9..30.1).contains(&days), "{days}");
    // 再申請一次 → 409
    assert_eq!(
        request_delete(&s, "shop-a", &s.owner, "shop-a").await.0,
        StatusCode::CONFLICT
    );
    // 店主收到通知(每位店主一封),稽核有紀錄
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM email_outbox WHERE subject LIKE '%已申請刪除%'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM audit_logs WHERE action = 'tenant.deletion_requested'"
        )
        .await,
        1
    );

    // 公開預約頁關閉;後台照常(店主要能取消);也不能再新增預約
    assert_eq!(public(&s.app).await, StatusCode::NOT_FOUND);
    // 顧客手上的預約管理連結也一併關閉(店家已經不營業了)
    assert_eq!(manage_status(&s.app).await, StatusCode::NOT_FOUND);
    let (status, me) = call(&s.app, Method::GET, "/t/shop-a/me", None, Some(&s.owner)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(me["deletion_scheduled_at"].is_string());
    let (status, body) = book(&s, "shop-a", at(day(4), 10), "d@example.com").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // 寬限期還沒到:背景任務不動它
    assert_eq!(retention_sweep(&pool, Some(730)).await.tenants_deleted, 0);

    // 取消 → 恢復營業
    assert_eq!(cancel_delete(&s, "shop-a").await.0, StatusCode::OK);
    assert_eq!(public(&s.app).await, StatusCode::OK);
    assert_eq!(manage_status(&s.app).await, StatusCode::OK);
    assert_eq!(
        book(&s, "shop-a", at(day(4), 10), "d@example.com").await.0,
        StatusCode::CREATED
    );
    assert_eq!(cancel_delete(&s, "shop-a").await.0, StatusCode::CONFLICT);
}

#[sqlx::test]
async fn an_expired_grace_period_deletes_the_shop_and_everything_in_it_but_nothing_else(
    pool: PgPool,
) {
    let s = shop(&pool, "shop-a", "a@example.com").await;
    let other = shop(&pool, "shop-b", "b@example.com").await;
    past_booking(&pool, &s, "shop-a", "c@example.com", 5, 0).await;
    past_booking(&pool, &other, "shop-b", "d@example.com", 5, 0).await;
    let (status, _) = request_delete(&s, "shop-a", &s.owner, "shop-a").await;
    assert_eq!(status, StatusCode::OK);
    let tenant_a: Uuid = sqlx::query_scalar("SELECT id FROM tenants WHERE slug = 'shop-a'")
        .fetch_one(&pool)
        .await
        .unwrap();

    sqlx::query("UPDATE tenants SET deletion_scheduled_at = now() - interval '1 minute' WHERE slug = 'shop-a'")
        .execute(&pool)
        .await
        .unwrap();
    let stats = retention_sweep(&pool, Some(730)).await;
    assert_eq!(stats.tenants_deleted, 1);

    // 這家店的所有資料都不見了
    for table in [
        "tenants WHERE id",
        "memberships WHERE tenant_id",
        "services WHERE tenant_id",
        "customers WHERE tenant_id",
        "bookings WHERE tenant_id",
        "audit_logs WHERE tenant_id",
        "email_outbox WHERE tenant_id",
    ] {
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} = $1"
        )))
        .bind(tenant_a)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    // 別家店、帳號本身都還在
    assert_eq!(count(&pool, "SELECT count(*) FROM tenants").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM bookings").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM users").await, 2);
    // 網址代稱釋出,可以重新註冊使用
    assert_eq!(
        create_tenant(&s.app, &s.owner, "shop-a").await.0,
        StatusCode::CREATED
    );
}

#[sqlx::test]
async fn a_shop_that_still_has_upcoming_bookings_at_the_deadline_is_not_deleted(pool: PgPool) {
    // 申請時沒有預約,但之後(競態或將來新增的入口)出現了:到期時不能直接把顧客在等的預約刪掉
    let s = shop(&pool, "shop-a", "a@example.com").await;
    assert_eq!(
        request_delete(&s, "shop-a", &s.owner, "shop-a").await.0,
        StatusCode::OK
    );
    let (status, booking) = {
        // 繞過公開與後台的檢查,直接寫入一筆未來的預約
        sqlx::query(
            "UPDATE tenants SET deletion_requested_at = NULL, deletion_scheduled_at = NULL",
        )
        .execute(&pool)
        .await
        .unwrap();
        let r = book(&s, "shop-a", at(day(3), 10), "c@example.com").await;
        sqlx::query(
            "UPDATE tenants SET deletion_requested_at = now() - interval '31 days',
                                deletion_scheduled_at = now() - interval '1 minute'",
        )
        .execute(&pool)
        .await
        .unwrap();
        r
    };
    assert_eq!(status, StatusCode::CREATED, "{booking}");

    let stats = retention_sweep(&pool, Some(730)).await;
    assert_eq!(stats.tenants_deleted, 0);
    assert_eq!(count(&pool, "SELECT count(*) FROM tenants").await, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM bookings").await, 1);
    // 刪除被自動取消,而且有稽核說明原因
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM tenants WHERE deletion_scheduled_at IS NOT NULL"
        )
        .await,
        0
    );
    let detail: Value = sqlx::query_scalar(
        "SELECT detail FROM audit_logs WHERE action = 'tenant.deletion_cancelled' AND actor_type = 'system'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(detail["reason"], "live_bookings");
}

#[sqlx::test]
async fn a_shop_whose_owner_is_gone_is_not_left_ownerless_by_the_safety_net(pool: PgPool) {
    // 店主申請刪除後刪了自己的帳號(成員資格停用)。到期時還有未來預約:
    // 不能「取消刪除」(那會留下一家沒有擁有者的店),要延後到預約結束再刪
    let s = shop(&pool, "shop-a", "a@example.com").await;
    let (status, booking) = book(&s, "shop-a", at(day(3), 10), "c@example.com").await;
    assert_eq!(status, StatusCode::CREATED, "{booking}");
    sqlx::query(
        "UPDATE tenants SET deletion_requested_at = now() - interval '31 days',
                            deletion_scheduled_at = now() - interval '1 minute'",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE memberships SET active = false WHERE role = 'owner'")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(retention_sweep(&pool, Some(730)).await.tenants_deleted, 0);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM tenants WHERE deletion_scheduled_at IS NOT NULL"
        )
        .await,
        1,
        "仍在排程刪除,沒有被取消"
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM audit_logs WHERE action = 'tenant.deletion_cancelled'"
        )
        .await,
        0
    );

    // 預約結束(這裡用取消代替)後,下一輪照原本的決定刪除
    sqlx::query("UPDATE bookings SET status = 'cancelled'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(retention_sweep(&pool, Some(730)).await.tenants_deleted, 1);
    assert_eq!(count(&pool, "SELECT count(*) FROM tenants").await, 0);
}

#[sqlx::test]
async fn the_deletion_mail_dedupe_key_does_not_contain_an_email_address(pool: PgPool) {
    // 信件寄出後內文會清空,但 dedupe_key 會一直留著:不能放個資
    let s = shop(&pool, "shop-a", "a@example.com").await;
    assert_eq!(
        request_delete(&s, "shop-a", &s.owner, "shop-a").await.0,
        StatusCode::OK
    );
    let key: String =
        sqlx::query_scalar("SELECT dedupe_key FROM email_outbox WHERE subject LIKE '%已申請刪除%'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!key.contains('@') && !key.contains("example.com"), "{key}");
}

#[sqlx::test]
async fn creating_a_booking_waits_for_a_deletion_request_in_progress(pool: PgPool) {
    // 「申請刪除」進行中(持有獨佔鎖)時,新增預約要等它結束,否則會留下一筆之後被刪掉的預約
    let s = shop(&pool, "shop-a", "a@example.com").await;
    let tenant: Uuid = sqlx::query_scalar("SELECT id FROM tenants")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut lock = pool.begin().await.unwrap();
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended('tenant-deletion:' || $1::text, 0))",
    )
    .bind(tenant)
    .execute(&mut *lock)
    .await
    .unwrap();

    let attempt = tokio::spawn({
        let (app, owner, service_id, owner_id) = (
            s.app.clone(),
            s.owner.clone(),
            s.service_id.clone(),
            s.owner_id,
        );
        async move {
            call(
                &app,
                Method::POST,
                "/t/shop-a/bookings",
                Some(json!({
                    "service_id": service_id,
                    "staff_id": owner_id,
                    "start": at(day(3), 10).to_rfc3339(),
                    "customer": {"name": "王小明", "email": "c@example.com"},
                })),
                Some(&owner),
            )
            .await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    assert!(!attempt.is_finished(), "新增預約應該在等鎖");

    lock.commit().await.unwrap();
    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(10), attempt)
        .await
        .expect("鎖釋放後要能完成")
        .unwrap();
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[sqlx::test]
async fn requesting_deletion_waits_for_bookings_being_created(pool: PgPool) {
    // 反方向:新增預約進行中(持有共享鎖)時,申請刪除要等它,等到之後才會看到那筆預約
    let s = shop(&pool, "shop-a", "a@example.com").await;
    let tenant: Uuid = sqlx::query_scalar("SELECT id FROM tenants")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut creating = pool.begin().await.unwrap();
    sqlx::query(
        "SELECT pg_advisory_xact_lock_shared(hashtextextended('tenant-deletion:' || $1::text, 0))",
    )
    .bind(tenant)
    .execute(&mut *creating)
    .await
    .unwrap();

    let mut requesting = pool.begin().await.unwrap();
    sqlx::query(
        "SELECT set_config('app.tenant_id', $1, true), set_config('app.user_id', $2, true)",
    )
    .bind(tenant.to_string())
    .bind(s.owner_id.to_string())
    .execute(&mut *requesting)
    .await
    .unwrap();
    sqlx::query("SET LOCAL lock_timeout = '300ms'")
        .execute(&mut *requesting)
        .await
        .unwrap();
    let err = sqlx::query("SELECT request_tenant_deletion(30)")
        .execute(&mut *requesting)
        .await
        .unwrap_err();
    // 55P03 = lock_not_available:代表它真的在等那把鎖
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("55P03")
    );
}

#[sqlx::test]
async fn stale_unverified_accounts_are_deleted_and_the_email_is_freed(pool: PgPool) {
    let s = shop(&pool, "shop-a", "a@example.com").await;
    let insert = |email: &'static str, verified: bool, age_days: i32| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO users (email, password_hash, name, created_at, email_verified_at)
                 VALUES ($1, 'x', '測試', now() - make_interval(days => $3),
                         CASE WHEN $2 THEN now() ELSE NULL END) RETURNING id",
            )
            .bind(email)
            .bind(verified)
            .bind(age_days)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let stale = insert("stale@example.com", false, 40).await;
    let _fresh = insert("fresh@example.com", false, 5).await;
    let _old_verified = insert("old-verified@example.com", true, 400).await;
    let member = insert("member@example.com", false, 40).await;
    sqlx::query(
        "INSERT INTO memberships (tenant_id, user_id, role) SELECT id, $1, 'staff' FROM tenants",
    )
    .bind(member)
    .execute(&pool)
    .await
    .unwrap();
    // 它的驗證連結、重設密碼連結與待寄信件
    sqlx::query("INSERT INTO email_verifications (user_id, token_hash, expires_at) VALUES ($1, 'h1', now() + interval '1 day')")
        .bind(stale)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO email_outbox (tenant_id, to_email, subject, body) VALUES (NULL, 'stale@example.com', '驗證', 'x')")
        .execute(&pool)
        .await
        .unwrap();

    let stats = retention_sweep(&pool, Some(730)).await;
    assert_eq!(stats.unverified_deleted, 1);
    let left: Vec<String> = sqlx::query_scalar(
        "SELECT email::text FROM users WHERE email LIKE '%@example.com' ORDER BY email",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        left,
        [
            "a@example.com",
            "fresh@example.com",
            "member@example.com",
            "old-verified@example.com"
        ]
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM email_verifications").await,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM email_outbox WHERE to_email = 'stale@example.com'"
        )
        .await,
        0
    );
    // 被占住的 Email 釋出了:真正的主人可以註冊
    assert_eq!(
        common::register(&s.app, "stale@example.com", "password123")
            .await
            .0,
        StatusCode::CREATED
    );
}

#[sqlx::test]
async fn stale_account_cleanup_refuses_a_period_that_would_hit_new_signups(pool: PgPool) {
    shop(&pool, "shop-a", "a@example.com").await;
    let err = sqlx::query("SELECT retention_delete_stale_unverified(1, 10)")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("22023")
    );
}

#[sqlx::test]
async fn an_active_subscription_blocks_deletion(pool: PgPool) {
    let s = shop(&pool, "shop-a", "a@example.com").await;
    sqlx::query(
        "INSERT INTO tenant_billing (tenant_id, stripe_customer_id, subscription_id, status)
         SELECT id, 'cus_1', 'sub_1', 'active' FROM tenants",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (status, body) = request_delete(&s, "shop-a", &s.owner, "shop-a").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("訂閱"));

    // 訂閱已取消就可以
    sqlx::query("UPDATE tenant_billing SET status = 'canceled'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request_delete(&s, "shop-a", &s.owner, "shop-a").await.0,
        StatusCode::OK
    );
}
