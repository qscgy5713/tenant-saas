mod common;

use axum::{
    Router,
    http::{Method, StatusCode},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Asia::Taipei;
use common::{add_member, app, call, create_service, create_tenant, set_plan, signup, user_id};
use serde_json::{Value, json};
use sqlx::PgPool;
use tenant_saas::availability::local_to_utc;
use uuid::Uuid;

fn at(date: NaiveDate, h: u32, m: u32) -> DateTime<Utc> {
    local_to_utc(
        Taipei,
        date,
        chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap(),
    )
    .unwrap()
}

fn day(offset: i64) -> NaiveDate {
    Utc::now().with_timezone(&Taipei).date_naive() + Duration::days(offset)
}

async fn plan_of(pool: &PgPool, slug: &str) -> String {
    sqlx::query_scalar("SELECT plan_id FROM tenants WHERE slug = $1")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// 開店並讓 owner 每天 09:00–18:00 提供一個 30 分鐘的服務,回傳 (app, owner token, owner id, service id)
async fn shop(pool: &PgPool) -> (Router, String, Uuid, String) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    assert_eq!(
        create_tenant(&app, &owner, "shop-a").await.0,
        StatusCode::CREATED
    );
    let service_id = create_service(&app, &owner, "shop-a", "剪髮").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let owner_id = user_id(pool, "a@example.com").await;
    let hours: Vec<Value> = (0..7)
        .map(|d| json!({"weekday": d, "start": "09:00", "end": "18:00"}))
        .collect();
    call(
        &app,
        Method::PUT,
        &format!("/t/shop-a/members/{owner_id}/working-hours"),
        Some(json!({"hours": hours})),
        Some(&owner),
    )
    .await;
    call(
        &app,
        Method::PUT,
        &format!("/t/shop-a/members/{owner_id}/services"),
        Some(json!({"service_ids": [service_id]})),
        Some(&owner),
    )
    .await;
    (app, owner, owner_id, service_id)
}

async fn book(
    app: &Router,
    owner: &str,
    service_id: &str,
    start: DateTime<Utc>,
    email: &str,
) -> StatusCode {
    call(app, Method::POST, "/t/shop-a/bookings", Some(json!({
        "service_id": service_id, "start": start.to_rfc3339(), "customer": {"name": "顧客", "email": email},
    })), Some(owner)).await.0
}

#[sqlx::test]
async fn new_shops_start_on_the_free_plan_and_expose_usage(pool: PgPool) {
    let (app, owner, _, _) = shop(&pool).await;
    assert_eq!(plan_of(&pool, "shop-a").await, "free");

    let (status, body) = call(&app, Method::GET, "/t/shop-a/plan", None, Some(&owner)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["plan"]["id"], "free");
    assert_eq!(body["plan"]["max_staff"], 2);
    assert_eq!(body["usage"]["staff"], 1);
    assert_eq!(body["usage"]["services"], 1);
    assert_eq!(body["usage"]["bookings_this_month"], 0);

    // 公開方案列表,依價格排序;企業版不限額 → null
    let (status, plans) = call(&app, Method::GET, "/plans", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = plans
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["free", "pro", "business"]);
    assert!(plans[2]["max_staff"].is_null());

    // 員工看不到方案與用量(含金額相關資訊)
    let staff = signup(&app, "s@example.com").await;
    add_member(&pool, "shop-a", "s@example.com", "staff").await;
    assert_eq!(
        call(&app, Method::GET, "/t/shop-a/plan", None, Some(&staff))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test]
async fn tenants_cannot_change_their_own_plan_or_limits(pool: PgPool) {
    let (_, _, owner_id, _) = shop(&pool).await;
    let tenant: Uuid = sqlx::query_scalar("SELECT id FROM tenants WHERE slug = 'shop-a'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut tx = tenant_saas::db::begin_scoped(&pool, Some(owner_id), Some(tenant))
        .await
        .unwrap();
    for sql in [
        "UPDATE tenants SET plan_id = 'business'",
        "UPDATE plans SET max_staff = NULL",
        "INSERT INTO plans (id, name) VALUES ('hack', 'x')",
        "DELETE FROM plans",
    ] {
        let err = sqlx::query(sql).execute(&mut *tx).await.unwrap_err();
        assert_eq!(
            err.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501"),
            "{sql}"
        );
        tx = tenant_saas::db::begin_scoped(&pool, Some(owner_id), Some(tenant))
            .await
            .unwrap();
    }
    assert_eq!(plan_of(&pool, "shop-a").await, "free");
}

#[sqlx::test]
async fn staff_limit_counts_members_and_pending_invitations(pool: PgPool) {
    let (app, owner, _, _) = shop(&pool).await;
    let invite = |email: &str| {
        let (app, owner, email) = (app.clone(), owner.clone(), email.to_string());
        async move {
            call(
                &app,
                Method::POST,
                "/t/shop-a/invitations",
                Some(json!({"email": email, "role": "staff"})),
                Some(&owner),
            )
            .await
        }
    };

    // 免費版 2 人:owner 1 + 一張邀請 = 2,第二張被擋
    let (status, first) = invite("b@example.com").await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = invite("c@example.com").await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert!(body["error"].as_str().unwrap().contains("免費版"));

    // 撤銷邀請會釋出名額
    let id = first["id"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/t/shop-a/invitations/{id}"),
            None,
            Some(&owner)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(invite("c@example.com").await.0, StatusCode::CREATED);

    // 升級後立刻生效
    set_plan(&pool, "shop-a", "pro").await;
    assert_eq!(invite("d@example.com").await.0, StatusCode::CREATED);
}

#[sqlx::test]
async fn concurrent_invitations_cannot_oversell_the_last_seat(pool: PgPool) {
    let (app, owner, _, _) = shop(&pool).await;
    let mut handles = Vec::new();
    for i in 0..8 {
        let (app, owner) = (app.clone(), owner.clone());
        handles.push(tokio::spawn(async move {
            call(
                &app,
                Method::POST,
                "/t/shop-a/invitations",
                Some(json!({"email": format!("u{i}@example.com"), "role": "staff"})),
                Some(&owner),
            )
            .await
            .0
        }));
    }
    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.unwrap());
    }
    let created = results
        .iter()
        .filter(|s| **s == StatusCode::CREATED)
        .count();
    assert_eq!(
        created, 1,
        "免費版剩 1 個名額,8 個併發請求只能成功 1 個: {results:?}"
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM invitations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test]
async fn accepting_an_invitation_rechecks_the_limit_after_a_downgrade(pool: PgPool) {
    let (app, owner, _, _) = shop(&pool).await;
    set_plan(&pool, "shop-a", "pro").await;
    let b = signup(&app, "b@example.com").await;
    let c = signup(&app, "c@example.com").await;
    let tok = |r: Value| r["token"].as_str().unwrap().to_string();
    let (_, ib) = call(
        &app,
        Method::POST,
        "/t/shop-a/invitations",
        Some(json!({"email": "b@example.com", "role": "staff"})),
        Some(&owner),
    )
    .await;
    let (_, ic) = call(
        &app,
        Method::POST,
        "/t/shop-a/invitations",
        Some(json!({"email": "c@example.com", "role": "staff"})),
        Some(&owner),
    )
    .await;

    // 邀請發出後被降級成免費版(上限 2):owner + 一位已加入 = 滿
    set_plan(&pool, "shop-a", "free").await;
    let accept = |token: &str, who: &str| {
        let (app, token, who) = (app.clone(), token.to_string(), who.to_string());
        async move {
            call(
                &app,
                Method::POST,
                "/invitations/accept",
                Some(json!({"token": token})),
                Some(&who),
            )
            .await
        }
    };
    assert_eq!(accept(&tok(ib), &b).await.0, StatusCode::OK);
    let ic_token = tok(ic);
    let (status, _) = accept(&ic_token, &c).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(
        call(&app, Method::GET, "/t/shop-a/me", None, Some(&c))
            .await
            .0,
        StatusCode::NOT_FOUND,
        "沒有被加進去"
    );

    // 邀請還在,方案升級後可以再接受
    set_plan(&pool, "shop-a", "pro").await;
    assert_eq!(accept(&ic_token, &c).await.0, StatusCode::OK);
}

#[sqlx::test]
async fn service_limit_includes_reactivation(pool: PgPool) {
    let (app, owner, _, first) = shop(&pool).await;
    // 免費版 5 項啟用中的服務:已有 1 項,再建 4 項
    let mut ids = vec![first];
    for i in 0..4 {
        ids.push(
            create_service(&app, &owner, "shop-a", &format!("服務{i}")).await["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    let create = || {
        let (app, owner) = (app.clone(), owner.clone());
        async move {
            call(
                &app,
                Method::POST,
                "/t/shop-a/services",
                Some(json!({"name": "第六項", "duration_minutes": 30})),
                Some(&owner),
            )
            .await
        }
    };
    assert_eq!(create().await.0, StatusCode::PAYMENT_REQUIRED);

    // 停用一項 → 可以再建一項;停用的不算
    let patch = |id: &str, active: bool| {
        let (app, owner, id) = (app.clone(), owner.clone(), id.to_string());
        async move {
            call(
                &app,
                Method::PATCH,
                &format!("/t/shop-a/services/{id}"),
                Some(json!({"active": active})),
                Some(&owner),
            )
            .await
            .0
        }
    };
    assert_eq!(patch(&ids[1], false).await, StatusCode::OK);
    assert_eq!(create().await.0, StatusCode::CREATED);
    // 又滿了:重新啟用剛才停用的那項,要被擋(否則停用再啟用就能繞過上限)
    assert_eq!(patch(&ids[1], true).await, StatusCode::PAYMENT_REQUIRED);
    // 對已啟用的服務重複送 active=true 不算新增
    assert_eq!(patch(&ids[0], true).await, StatusCode::OK);
    // 改名等不影響啟用數的修改,不受限額影響
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/t/shop-a/services/{}", ids[0]),
        Some(json!({"name": "改名"})),
        Some(&owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test]
async fn monthly_booking_limit_counts_by_the_shops_calendar_month(pool: PgPool) {
    let (app, owner, _, service) = shop(&pool).await;
    sqlx::query("UPDATE plans SET max_bookings_per_month = 3 WHERE id = 'free'")
        .execute(&pool)
        .await
        .unwrap();

    // 找一個「下個月」和「再下個月」的日期,避免被「必須在未來 90 天內」限制擋到
    let base = day(5);
    let this_month = at(base, 10, 0);
    for h in 0..3 {
        assert_eq!(
            book(
                &app,
                &owner,
                &service,
                this_month + Duration::hours(h),
                &format!("c{h}@example.com")
            )
            .await,
            StatusCode::CREATED
        );
    }
    let (status, body) = call(&app, Method::POST, "/t/shop-a/bookings", Some(json!({
        "service_id": service, "start": (this_month + Duration::hours(3)).to_rfc3339(), "customer": {"name": "x", "email": "x@example.com"},
    })), Some(&owner)).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");

    // 取消一筆會釋出額度;已取消的不算
    let (_, list) = call(
        &app,
        Method::GET,
        "/t/shop-a/bookings?status=confirmed",
        None,
        Some(&owner),
    )
    .await;
    let id = list["items"][0]["id"].as_str().unwrap();
    call(
        &app,
        Method::PATCH,
        &format!("/t/shop-a/bookings/{id}"),
        Some(json!({"status": "cancelled"})),
        Some(&owner),
    )
    .await;
    assert_eq!(
        book(
            &app,
            &owner,
            &service,
            this_month + Duration::hours(3),
            "x@example.com"
        )
        .await,
        StatusCode::CREATED
    );

    // 不同月份各自計算:兩個月後的某天不受影響(同樣會滿,但是獨立計算)
    let later = at(base + Duration::days(40), 10, 0);
    assert_eq!(
        book(&app, &owner, &service, later, "y@example.com").await,
        StatusCode::CREATED
    );
    // 40 天後一定落在不同的當地月份,兩個月份各自獨立計數
    let count_in_month_of = |at: DateTime<Utc>| {
        let pool = pool.clone();
        let (start, end) = tenant_saas::plan::month_bounds(Taipei, at);
        async move {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM bookings WHERE status = 'confirmed' AND starts_at >= $1 AND starts_at < $2")
                .bind(start).bind(end).fetch_one(&pool).await.unwrap()
        }
    };
    assert_eq!(count_in_month_of(this_month).await, 3);
    assert_eq!(count_in_month_of(later).await, 1);
}

#[sqlx::test]
async fn concurrent_bookings_cannot_oversell_the_monthly_limit(pool: PgPool) {
    let (app, owner, owner_id, service) = shop(&pool).await;
    // 兩位員工才排得開同時段的併發;限額設成 2,10 個併發請求只能成功 2 個
    let second = signup(&app, "b@example.com").await;
    add_member(&pool, "shop-a", "b@example.com", "staff").await;
    let b_id = user_id(&pool, "b@example.com").await;
    for id in [owner_id, b_id] {
        let hours: Vec<Value> = (0..7)
            .map(|d| json!({"weekday": d, "start": "09:00", "end": "18:00"}))
            .collect();
        call(
            &app,
            Method::PUT,
            &format!("/t/shop-a/members/{id}/working-hours"),
            Some(json!({"hours": hours})),
            Some(&owner),
        )
        .await;
        call(
            &app,
            Method::PUT,
            &format!("/t/shop-a/members/{id}/services"),
            Some(json!({"service_ids": [service]})),
            Some(&owner),
        )
        .await;
    }
    let _ = second;
    sqlx::query("UPDATE plans SET max_bookings_per_month = 2 WHERE id = 'free'")
        .execute(&pool)
        .await
        .unwrap();

    let base = at(day(5), 9, 0);
    let mut handles = Vec::new();
    for i in 0..10 {
        let (app, owner, service) = (app.clone(), owner.clone(), service.clone());
        // 每個請求不同時段,排除約束不會幫忙擋 → 只有額度檢查能擋
        let start = base + Duration::minutes(30 * i);
        handles.push(tokio::spawn(async move {
            book(&app, &owner, &service, start, &format!("c{i}@example.com")).await
        }));
    }
    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.unwrap());
    }
    let created = results
        .iter()
        .filter(|s| **s == StatusCode::CREATED)
        .count();
    let blocked = results
        .iter()
        .filter(|s| **s == StatusCode::PAYMENT_REQUIRED)
        .count();
    assert_eq!((created, blocked), (2, 8), "{results:?}");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM bookings WHERE status = 'confirmed'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 2);
}

#[sqlx::test]
async fn customers_see_a_friendly_message_when_the_shop_is_full(pool: PgPool) {
    let (app, owner, _, service) = shop(&pool).await;
    sqlx::query("UPDATE plans SET max_bookings_per_month = 1 WHERE id = 'free'")
        .execute(&pool)
        .await
        .unwrap();
    let start = at(day(5), 10, 0);
    assert_eq!(
        book(&app, &owner, &service, start, "first@example.com").await,
        StatusCode::CREATED
    );

    // 額滿後,顧客自助預約的申請就被擋下(不寄驗證信、不留待確認紀錄);
    // 顧客只看到「額滿」,看不到店家的方案名稱與上限
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM email_outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (status, body) = call(&app, Method::POST, "/public/shops/shop-a/bookings", Some(json!({
        "service_id": service, "start": at(day(5), 12, 0).to_rfc3339(), "customer": {"name": "顧客", "email": "late@example.com"},
    })), None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let message = body["error"].as_str().unwrap();
    assert!(message.contains("額滿"));
    assert!(
        !message.contains("免費版") && !message.contains("升級") && !message.contains('1'),
        "不可洩漏方案資訊: {message}"
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM email_outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM bookings WHERE status = 'pending'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(pending, 0);
}

#[sqlx::test]
async fn confirming_counts_toward_the_limit(pool: PgPool) {
    let (app, owner, _, service) = shop(&pool).await;
    sqlx::query("UPDATE plans SET max_bookings_per_month = 1 WHERE id = 'free'")
        .execute(&pool)
        .await
        .unwrap();
    // 兩位顧客在額度還有時都能申請(待確認不占額度)
    let start = at(day(5), 10, 0);
    let request = |email: &'static str, start: DateTime<Utc>| {
        let (app, service) = (app.clone(), service.clone());
        async move {
            call(&app, Method::POST, "/public/shops/shop-a/bookings", Some(json!({
            "service_id": service, "start": start.to_rfc3339(), "customer": {"name": "顧客", "email": email},
        })), None).await.0
        }
    };
    assert_eq!(request("a@c.example", start).await, StatusCode::CREATED);
    assert_eq!(
        request("b@c.example", start + Duration::hours(2)).await,
        StatusCode::CREATED
    );

    // 用 worker 寄信取出 token
    let memory = tenant_saas::mail::MemoryMailer::new();
    tenant_saas::worker::tick(
        &pool,
        &tenant_saas::mail::Mailer::Memory(memory.clone()),
        "http://app.test",
    )
    .await
    .unwrap();
    let token_of = |email: &str| -> String {
        let mail = memory.sent().into_iter().find(|m| m.to == email).unwrap();
        mail.body
            .split("/bookings/")
            .nth(1)
            .unwrap()
            .chars()
            .take(64)
            .collect()
    };
    let _ = &owner;
    assert_eq!(
        call(
            &app,
            Method::POST,
            &format!("/public/bookings/{}/confirm", token_of("a@c.example")),
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    // 第二位按確認時額度已滿:告知額滿,且那筆被取消而不是一直掛著
    let (status, body) = call(
        &app,
        Method::POST,
        &format!("/public/bookings/{}/confirm", token_of("b@c.example")),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("額滿"));
    let st: String = sqlx::query_scalar("SELECT b.status::text FROM bookings b JOIN customers c ON c.id = b.customer_id WHERE c.email = 'b@c.example'").fetch_one(&pool).await.unwrap();
    assert_eq!(st, "cancelled");
}

/// 確定性地證明「這些操作真的會先拿到限額鎖」:測試先持有同一把鎖,
/// 請求必須被擋住;放開之後才能完成。比起併發賽跑(視運氣才會暴露超賣)可靠得多。
#[sqlx::test]
async fn quota_checks_take_the_per_tenant_lock(pool: PgPool) {
    let (app, owner, _, service) = shop(&pool).await;
    set_plan(&pool, "shop-a", "business").await; // 額度不是重點,這裡只看會不會等鎖
    let tenant: Uuid = sqlx::query_scalar("SELECT id FROM tenants WHERE slug = 'shop-a'")
        .fetch_one(&pool)
        .await
        .unwrap();

    let start = at(day(5), 10, 0);
    let cases: Vec<(&str, &str, Value)> = vec![
        (
            "staff",
            "/t/shop-a/invitations",
            json!({"email": "n@example.com", "role": "staff"}),
        ),
        (
            "services",
            "/t/shop-a/services",
            json!({"name": "新服務", "duration_minutes": 30}),
        ),
        (
            "bookings",
            "/t/shop-a/bookings",
            json!({
                "service_id": service, "start": start.to_rfc3339(), "customer": {"name": "顧客", "email": "c@example.com"},
            }),
        ),
    ];

    for (kind, uri, body) in cases {
        // 先持有這家店該資源的鎖(key 格式是 plan::lock 與 accept_invitation() 之間的約定)
        let mut guard = pool.begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("quota:{kind}:{tenant}"))
            .execute(&mut *guard)
            .await
            .unwrap();

        let (app, owner, uri) = (app.clone(), owner.clone(), uri.to_string());
        let mut request = tokio::spawn(async move {
            call(&app, Method::POST, &uri, Some(body), Some(&owner))
                .await
                .0
        });

        let blocked =
            tokio::time::timeout(std::time::Duration::from_millis(400), &mut request).await;
        assert!(blocked.is_err(), "{kind}:鎖被別人持有時,請求必須等待");

        guard.commit().await.unwrap();
        let status = tokio::time::timeout(std::time::Duration::from_secs(5), request)
            .await
            .unwrap_or_else(|_| panic!("{kind}:放開鎖後請求應該完成"))
            .unwrap();
        assert_eq!(status, StatusCode::CREATED, "{kind}");
    }
}

/// 反向確認:鎖是「每家店、每種資源」各一把,不會誤擋別家店或別種操作
#[sqlx::test]
async fn quota_locks_are_scoped_per_tenant_and_resource(pool: PgPool) {
    let (app, owner, _, _) = shop(&pool).await;
    let tenant: Uuid = sqlx::query_scalar("SELECT id FROM tenants WHERE slug = 'shop-a'")
        .fetch_one(&pool)
        .await
        .unwrap();

    // 持有「別家店的 services 鎖」與「本店的 bookings 鎖」,新增本店服務不該被擋
    let mut guard = pool.begin().await.unwrap();
    for key in [
        format!("quota:services:{}", Uuid::new_v4()),
        format!("quota:bookings:{tenant}"),
    ] {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(key)
            .execute(&mut *guard)
            .await
            .unwrap();
    }
    let created = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        call(
            &app,
            Method::POST,
            "/t/shop-a/services",
            Some(json!({"name": "不受影響", "duration_minutes": 30})),
            Some(&owner),
        ),
    )
    .await
    .expect("不相干的鎖不該擋住請求");
    assert_eq!(created.0, StatusCode::CREATED);
    guard.rollback().await.unwrap();
}

#[sqlx::test]
async fn rescheduling_into_another_month_counts_toward_that_months_limit(pool: PgPool) {
    let (app, owner, _, service) = shop(&pool).await;
    sqlx::query("UPDATE plans SET max_bookings_per_month = 1 WHERE id = 'free'")
        .execute(&pool)
        .await
        .unwrap();

    let first = at(day(5), 10, 0); // 占滿這個月份唯一的名額
    let other_month = at(day(5) + Duration::days(40), 10, 0);
    assert_eq!(
        book(&app, &owner, &service, first, "a@example.com").await,
        StatusCode::CREATED
    );
    let (status, created) = call(&app, Method::POST, "/t/shop-a/bookings", Some(json!({
        "service_id": service, "start": other_month.to_rfc3339(), "customer": {"name": "顧客", "email": "b@example.com"},
    })), Some(&owner)).await;
    assert_eq!(status, StatusCode::CREATED, "不同月份各有各的名額");
    let token = created["manage_token"].as_str().unwrap();
    let reschedule = |to: DateTime<Utc>| {
        let (app, uri) = (app.clone(), format!("/public/bookings/{token}/reschedule"));
        async move {
            call(
                &app,
                Method::POST,
                &uri,
                Some(json!({"start": to.to_rfc3339()})),
                None,
            )
            .await
        }
    };

    // 改到已額滿的月份 → 擋下;顧客只看到「額滿」
    let (status, body) = reschedule(first + Duration::hours(2)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("額滿"));
    // 同月份內改期不增加用量,不受影響
    assert_eq!(
        reschedule(other_month + Duration::hours(2)).await.0,
        StatusCode::OK
    );
}

/// SQL 函式 accept_invitation() 與 Rust 的 plan::lock 必須用同一把鎖(key 對不上的話鎖形同虛設)
#[sqlx::test]
async fn accept_invitation_uses_the_same_staff_lock_as_inviting(pool: PgPool) {
    let (app, owner, _, _) = shop(&pool).await;
    set_plan(&pool, "shop-a", "pro").await;
    let invitee = signup(&app, "b@example.com").await;
    let (_, inv) = call(
        &app,
        Method::POST,
        "/t/shop-a/invitations",
        Some(json!({"email": "b@example.com", "role": "staff"})),
        Some(&owner),
    )
    .await;
    let token = inv["token"].as_str().unwrap().to_string();
    let tenant: Uuid = sqlx::query_scalar("SELECT id FROM tenants WHERE slug = 'shop-a'")
        .fetch_one(&pool)
        .await
        .unwrap();

    let mut guard = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("quota:staff:{tenant}"))
        .execute(&mut *guard)
        .await
        .unwrap();

    let mut request = {
        let app = app.clone();
        tokio::spawn(async move {
            call(
                &app,
                Method::POST,
                "/invitations/accept",
                Some(json!({"token": token})),
                Some(&invitee),
            )
            .await
            .0
        })
    };
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(400), &mut request)
            .await
            .is_err(),
        "接受邀請必須等同一把鎖"
    );
    guard.commit().await.unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), request)
            .await
            .unwrap()
            .unwrap(),
        StatusCode::OK
    );
}
