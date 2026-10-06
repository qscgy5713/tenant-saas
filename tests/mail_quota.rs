//! 每位收件者每小時的寄信上限(跨店家)與邀請端點的限流。

mod common;

use axum::{Router, http::Method, http::StatusCode};
use common::{
    add_member, app_with_auth_limit, call, create_service, create_tenant, signup, test_config,
    user_id,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tenant_saas::{
    config::Config,
    routes::{self, AppState},
};

fn app_with_mail_limit(pool: PgPool, limit: u32) -> Router {
    let config = Config {
        max_mails_per_recipient_per_hour: limit,
        ..test_config(3600)
    };
    routes::router(AppState::new(pool, &config))
}

/// 開一家店,owner 每天 09:00–18:00 提供一個 60 分鐘的服務。回傳 (owner token, 服務 id)
async fn open_shop(app: &Router, pool: &PgPool, slug: &str, owner_email: &str) -> (String, String) {
    let owner = signup(app, owner_email).await;
    create_tenant(app, &owner, slug).await;
    let service = create_service(app, &owner, slug, "剪髮").await;
    let service_id = service["id"].as_str().unwrap().to_string();
    call(
        app,
        Method::PATCH,
        &format!("/t/{slug}/services/{service_id}"),
        Some(json!({"duration_minutes": 60})),
        Some(&owner),
    )
    .await;
    let id = user_id(pool, owner_email).await;
    let hours: Vec<Value> = (0..7)
        .map(|d| json!({"weekday": d, "start": "09:00", "end": "18:00"}))
        .collect();
    call(
        app,
        Method::PUT,
        &format!("/t/{slug}/members/{id}/working-hours"),
        Some(json!({"hours": hours})),
        Some(&owner),
    )
    .await;
    call(
        app,
        Method::PUT,
        &format!("/t/{slug}/members/{id}/services"),
        Some(json!({"service_ids": [service_id]})),
        Some(&owner),
    )
    .await;
    (owner, service_id)
}

/// 對公開預約頁送出一筆申請(第 `n` 天的 10:00 之後,避免不同申請撞時段)
async fn request(app: &Router, slug: &str, service_id: &str, email: &str, n: i64) -> StatusCode {
    let day = chrono::Utc::now().date_naive() + chrono::Duration::days(3 + n);
    let start = format!("{day}T03:00:00Z"); // 台北 11:00
    call(
        app,
        Method::POST,
        &format!("/public/shops/{slug}/bookings"),
        Some(json!({
            "service_id": service_id, "start": start,
            "customer": {"name": "顧客", "email": email},
        })),
        None,
    )
    .await
    .0
}

async fn mails_to(pool: &PgPool, email: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM email_outbox WHERE lower(to_email) = lower($1)")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn one_recipient_cannot_be_flooded_across_different_shops(pool: PgPool) {
    let app = app_with_mail_limit(pool.clone(), 3);
    let victim = "victim@example.com";
    let mut shops = Vec::new();
    for i in 0..5 {
        let slug = format!("shop-{i}");
        let (_, service) = open_shop(&app, &pool, &slug, &format!("owner{i}@example.com")).await;
        shops.push((slug, service));
    }

    // 每家店各一筆,單一店家內的「最多 5 筆」限制擋不住;跨店家的上限在第 4 封擋下
    let mut statuses = Vec::new();
    for (i, (slug, service)) in shops.iter().enumerate() {
        statuses.push(request(&app, slug, service, victim, i as i64).await);
    }
    assert_eq!(
        statuses,
        [
            StatusCode::CREATED,
            StatusCode::CREATED,
            StatusCode::CREATED,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::TOO_MANY_REQUESTS,
        ]
    );
    assert_eq!(mails_to(&pool, victim).await, 3);

    // 被擋下的申請不能留下半成品(預約、客戶資料都沒有)
    let (bookings, customers): (i64, i64) =
        sqlx::query_as("SELECT (SELECT count(*) FROM bookings), (SELECT count(*) FROM customers)")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((bookings, customers), (3, 3));

    // Email 大小寫不同也算同一個收件者;別的收件者不受影響
    let (slug, service) = &shops[3];
    assert_eq!(
        request(&app, slug, service, "VICTIM@Example.com", 10).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        request(&app, slug, service, "someone-else@example.com", 11).await,
        StatusCode::CREATED
    );
}

#[sqlx::test]
async fn the_hour_window_slides(pool: PgPool) {
    let app = app_with_mail_limit(pool.clone(), 1);
    let (_, service) = open_shop(&app, &pool, "shop-a", "a@example.com").await;
    assert_eq!(
        request(&app, "shop-a", &service, "c@example.com", 0).await,
        StatusCode::CREATED
    );
    assert_eq!(
        request(&app, "shop-a", &service, "c@example.com", 1).await,
        StatusCode::TOO_MANY_REQUESTS
    );

    // 超過一小時後又可以收信
    sqlx::query("UPDATE email_outbox SET created_at = now() - interval '61 minutes'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(&app, "shop-a", &service, "c@example.com", 1).await,
        StatusCode::CREATED
    );
}

#[sqlx::test]
async fn simultaneous_requests_cannot_slip_past_the_limit(pool: PgPool) {
    let app = app_with_mail_limit(pool.clone(), 2);
    let (_, service) = open_shop(&app, &pool, "shop-a", "a@example.com").await;
    let tasks: Vec<_> = (0..8)
        .map(|i| {
            let (app, service) = (app.clone(), service.clone());
            tokio::spawn(async move { request(&app, "shop-a", &service, "c@example.com", i).await })
        })
        .collect();
    let mut statuses = Vec::new();
    for t in tasks {
        statuses.push(t.await.unwrap());
    }
    let created = statuses
        .iter()
        .filter(|s| **s == StatusCode::CREATED)
        .count();
    assert_eq!(created, 2, "{statuses:?}");
    assert_eq!(mails_to(&pool, "c@example.com").await, 2);
}

/// 只有「寄給外部指定的收件者」才受限;系統對自己的使用者寄的信(重設密碼、鎖定通知)不在此限
#[sqlx::test]
async fn invitations_and_staff_bookings_are_limited_too_but_account_mail_is_not(pool: PgPool) {
    let app = app_with_mail_limit(pool.clone(), 1);
    let (owner, service) = open_shop(&app, &pool, "shop-a", "a@example.com").await;

    // 店家代客預約:同一位顧客第二次被擋
    let book = |n: i64| {
        let (app, owner, service) = (app.clone(), owner.clone(), service.clone());
        async move {
            let day = chrono::Utc::now().date_naive() + chrono::Duration::days(3 + n);
            call(
                &app,
                Method::POST,
                "/t/shop-a/bookings",
                Some(json!({
                    "service_id": service, "start": format!("{day}T03:00:00Z"),
                    "customer": {"name": "顧客", "email": "c@example.com"},
                })),
                Some(&owner),
            )
            .await
            .0
        }
    };
    assert_eq!(book(0).await, StatusCode::CREATED);
    assert_eq!(book(1).await, StatusCode::TOO_MANY_REQUESTS);

    // 邀請:同一個 Email 第二次被擋(先升級方案,免得先撞到成員人數上限)
    common::set_plan(&pool, "shop-a", "business").await;
    let invite = |email: &'static str| {
        let (app, owner) = (app.clone(), owner.clone());
        async move {
            call(
                &app,
                Method::POST,
                "/t/shop-a/invitations",
                Some(json!({"email": email, "role": "staff"})),
                Some(&owner),
            )
            .await
            .0
        }
    };
    assert_eq!(invite("new@example.com").await, StatusCode::CREATED);
    // 被擋下的邀請不能留下半成品:沒有邀請紀錄,之後(窗口過去)可以再邀
    sqlx::query("DELETE FROM invitations WHERE email = 'new@example.com'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        invite("new@example.com").await,
        StatusCode::TOO_MANY_REQUESTS
    );
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM invitations WHERE email = 'new@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 0);

    // 忘記密碼寄給「已註冊的使用者」,不受這個上限影響(它有自己的每小時 3 封)
    for _ in 0..2 {
        let (status, _) = call(
            &app,
            Method::POST,
            "/auth/forgot-password",
            Some(json!({"email": "a@example.com"})),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }
    assert_eq!(mails_to(&pool, "a@example.com").await, 2);
    let _ = add_member;
}

#[sqlx::test]
async fn resend_over_the_limit_looks_exactly_like_any_other_silent_refusal(pool: PgPool) {
    let app = app_with_mail_limit(pool.clone(), 1);
    let (_, service) = open_shop(&app, &pool, "shop-a", "a@example.com").await;
    let day = chrono::Utc::now().date_naive() + chrono::Duration::days(3);
    let (_, created) = call(
        &app,
        Method::POST,
        "/public/shops/shop-a/bookings",
        Some(json!({
            "service_id": service, "start": format!("{day}T03:00:00Z"),
            "customer": {"name": "顧客", "email": "c@example.com"},
        })),
        None,
    )
    .await;
    let id = created["id"].as_str().unwrap();
    sqlx::query("UPDATE bookings SET verification_sent_at = now() - interval '5 minutes'")
        .execute(&pool)
        .await
        .unwrap();

    let resend = |email: &str, id: &str| {
        let (app, email, id) = (app.clone(), email.to_string(), id.to_string());
        async move {
            call(
                &app,
                Method::POST,
                &format!("/public/shops/shop-a/bookings/{id}/resend"),
                Some(json!({"email": email})),
                None,
            )
            .await
        }
    };
    // 額度已用完(上面那封驗證信):重寄被靜默拒絕,回應與「預約不存在」一模一樣
    let over = resend("c@example.com", id).await;
    let nobody = resend("c@example.com", &uuid::Uuid::new_v4().to_string()).await;
    assert_eq!(over, nobody);
    assert_eq!(
        over.0,
        StatusCode::ACCEPTED,
        "端點是真的存在,不是兩次都 404: {over:?}"
    );
    assert_eq!(mails_to(&pool, "c@example.com").await, 1);

    // 對照:額度放寬的話,同一個請求確實會寄出新的信 —— 證明上面的「沒寄」是額度造成的,
    // 不是因為重寄在其他條件下本來就不會寄
    let relaxed = app_with_mail_limit(pool.clone(), 100);
    let (status, _) = call(
        &relaxed,
        Method::POST,
        &format!("/public/shops/shop-a/bookings/{id}/resend"),
        Some(json!({"email": "c@example.com"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(mails_to(&pool, "c@example.com").await, 2);
}

#[sqlx::test]
async fn accepting_invitations_shares_the_auth_rate_limit(pool: PgPool) {
    let app = app_with_auth_limit(pool, 3);
    let token = signup(&app, "a@example.com").await;
    let mut statuses = Vec::new();
    for _ in 0..5 {
        statuses.push(
            call(
                &app,
                Method::POST,
                "/invitations/accept",
                Some(json!({"token": "0".repeat(64)})),
                Some(&token),
            )
            .await
            .0,
        );
    }
    // signup 已用掉 1 次額度(同一個來源):前 2 次到達端點(無效的邀請 = 404),之後被限流
    assert_eq!(
        statuses,
        [
            StatusCode::NOT_FOUND,
            StatusCode::NOT_FOUND,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::TOO_MANY_REQUESTS,
        ]
    );
}

/// 直接、確定性地驗證鎖:第一個交易(已通過檢查、已寫入信件,但還沒提交)持有該收件者的鎖,
/// 第二個交易對同一個收件者做檢查必須等它;第一個提交後,第二個看得到那封信,所以被拒絕。
/// (上面「同時 8 個請求」打的是同一家店,而同一家店的預約建立本來就會因為每月額度的鎖排隊,
/// 測不到這個鎖 —— 拿掉鎖那個突變曾經在那個測試下存活。)
#[sqlx::test]
async fn the_recipient_lock_serializes_the_check_and_the_write(pool: PgPool) {
    let mut first = pool.begin().await.unwrap();
    let ok: bool = sqlx::query_scalar("SELECT mail_quota_ok('c@example.com', 1)")
        .fetch_one(&mut *first)
        .await
        .unwrap();
    assert!(ok);
    sqlx::query("INSERT INTO email_outbox (tenant_id, to_email, subject, body) VALUES (NULL, 'c@example.com', 's', 'b')")
        .execute(&mut *first)
        .await
        .unwrap();

    let pool2 = pool.clone();
    let second = tokio::spawn(async move {
        let mut tx = pool2.begin().await.unwrap();
        sqlx::query_scalar::<_, bool>("SELECT mail_quota_ok('C@EXAMPLE.com', 1)")
            .fetch_one(&mut *tx)
            .await
            .unwrap()
    });
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(!second.is_finished(), "第二個檢查應該卡在等第一個交易的鎖");

    // 不同收件者不會被卡住
    let mut other = pool.begin().await.unwrap();
    let ok: bool = sqlx::query_scalar("SELECT mail_quota_ok('other@example.com', 1)")
        .fetch_one(&mut *other)
        .await
        .unwrap();
    assert!(ok);

    first.commit().await.unwrap();
    assert!(
        !second.await.unwrap(),
        "第一個提交後,第二個看得到那封信 → 超過上限"
    );
}
