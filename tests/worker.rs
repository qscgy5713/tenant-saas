mod common;

use std::time::Duration as StdDuration;

use axum::{
    Router,
    http::{Method, StatusCode},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Asia::Taipei;
use common::{add_member, app, call, create_service, create_tenant, signup, user_id};
use serde_json::{Value, json};
use sqlx::PgPool;
use tenant_saas::{
    availability::local_to_utc,
    db::{begin_scoped, begin_worker},
    mail::{Mailer, MemoryMailer},
    worker::{self, TickStats, tick},
};
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

struct Shop {
    app: Router,
    owner: String,
    owner_id: Uuid,
    service_id: String,
    tenant_id: Uuid,
}

/// 開店 shop-a(台北時區),owner 每天 09:00–18:00 提供 60 分鐘的服務
async fn shop(pool: &PgPool) -> Shop {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    let service_id = create_service(&app, &owner, "shop-a", "剪髮").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    call(
        &app,
        Method::PATCH,
        &format!("/t/shop-a/services/{service_id}"),
        Some(json!({"duration_minutes": 60})),
        Some(&owner),
    )
    .await;
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
    let tenant_id = sqlx::query_scalar("SELECT id FROM tenants WHERE slug = 'shop-a'")
        .fetch_one(pool)
        .await
        .unwrap();
    Shop {
        app,
        owner,
        owner_id,
        service_id,
        tenant_id,
    }
}

fn mailer() -> (Mailer, MemoryMailer) {
    let memory = MemoryMailer::new();
    (Mailer::Memory(memory.clone()), memory)
}

/// 顧客自助預約(公開端點),回傳 (狀態, 回應)
async fn request(s: &Shop, start: DateTime<Utc>, email: &str) -> (StatusCode, Value) {
    call(
        &s.app,
        Method::POST,
        "/public/shops/shop-a/bookings",
        Some(json!({
            "service_id": s.service_id, "start": start.to_rfc3339(),
            "customer": {"name": "顧客", "email": email},
        })),
        None,
    )
    .await
}

/// 從信件內容取出連結中的 64 字元十六進位 token
fn token_in(body: &str) -> String {
    let after = body
        .split("/bookings/")
        .nth(1)
        .unwrap_or_else(|| panic!("信裡沒有預約連結: {body}"));
    after.chars().take(64).collect()
}

async fn slot_available(s: &Shop, d: NaiveDate, start: DateTime<Utc>) -> bool {
    let uri = format!(
        "/public/shops/shop-a/availability?service_id={}&from={d}",
        s.service_id
    );
    let (_, body) = call(&s.app, Method::GET, &uri, None, None).await;
    body["slots"].as_array().unwrap().iter().any(|x| {
        x["start"]
            .as_str()
            .unwrap()
            .parse::<DateTime<Utc>>()
            .unwrap()
            == start
    })
}

async fn status_of(pool: &PgPool, email: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT b.status::text FROM bookings b JOIN customers c ON c.id = b.customer_id WHERE c.email = $1::citext ORDER BY b.created_at")
        .bind(email).fetch_all(pool).await.unwrap()
}

// ---------- 確認流程 ----------

#[sqlx::test]
async fn public_booking_needs_email_confirmation(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let d = day(3);
    let start = at(d, 10, 0);

    let (status, body) = request(&s, start, "c@example.com").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["status"], "pending");
    assert!(
        body.get("manage_token").is_none(),
        "token 不能出現在回應裡,否則可以替別人的 Email 確認"
    );
    assert!(
        !body
            .to_string()
            .chars()
            .collect::<Vec<_>>()
            .windows(64)
            .any(|w| w.iter().all(char::is_ascii_hexdigit))
    );

    // 待確認不占時段
    assert!(slot_available(&s, d, start).await);
    assert_eq!(status_of(&pool, "c@example.com").await, vec!["pending"]);

    // 信已排進 outbox,但寄出前信箱收不到
    assert!(memory.sent().is_empty());
    let stats = tick(&pool, &mailer, "http://app.test").await.unwrap();
    assert_eq!(stats.sent, 1);
    let mails = memory.sent();
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to, "c@example.com");
    assert!(mails[0].subject.contains("店 shop-a") || mails[0].subject.contains("shop-a"));
    let token = token_in(&mails[0].body);

    // 寄出後 outbox 不保留含 token 的內容
    let (status, kept): (String, String) = sqlx::query_as("SELECT status, body FROM email_outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((status.as_str(), kept.as_str()), ("sent", ""));

    // 確認前:查得到、是 pending;亂猜的 token 404
    let (_, view) = call(
        &s.app,
        Method::GET,
        &format!("/public/bookings/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(view["status"], "pending");
    assert_eq!(
        call(
            &s.app,
            Method::POST,
            &format!("/public/bookings/{}/confirm", "0".repeat(64)),
            None,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    // 確認:時段此時才被占住
    let (status, view) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/confirm"),
        None,
        None,
    )
    .await;
    assert_eq!(
        (status, view["status"].as_str()),
        (StatusCode::OK, Some("confirmed"))
    );
    assert!(!slot_available(&s, d, start).await);

    // 重複確認視為成功,且不會重複寄確認信
    assert_eq!(
        call(
            &s.app,
            Method::POST,
            &format!("/public/bookings/{token}/confirm"),
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    let mails = memory.sent();
    assert_eq!(mails.len(), 2, "驗證信 + 一封確認信");
    assert!(mails[1].subject.contains("已確認"));
    assert_eq!(token_in(&mails[1].body), token, "確認信帶有管理連結");
}

#[sqlx::test]
async fn first_confirmation_wins_when_two_people_request_the_same_slot(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let start = at(day(3), 10, 0);

    // 兩人都能提出申請(待確認不占時段)
    assert_eq!(
        request(&s, start, "first@example.com").await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        request(&s, start, "second@example.com").await.0,
        StatusCode::CREATED
    );
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    let mails = memory.sent();
    let token_of = |email: &str| token_in(&mails.iter().find(|m| m.to == email).unwrap().body);
    let (t1, t2) = (
        token_of("first@example.com"),
        token_of("second@example.com"),
    );

    assert_eq!(
        call(
            &s.app,
            Method::POST,
            &format!("/public/bookings/{t1}/confirm"),
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, _) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{t2}/confirm"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        status_of(&pool, "second@example.com").await,
        vec!["cancelled"],
        "輸的那筆要被取消,不能一直掛著"
    );
    assert_eq!(
        status_of(&pool, "first@example.com").await,
        vec!["confirmed"]
    );
}

#[sqlx::test]
async fn confirmation_links_expire_and_pending_does_not_block(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let start = at(day(3), 10, 0);

    request(&s, start, "late@example.com").await;
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    let token = token_in(&memory.sent()[0].body);

    // 待確認不影響員工代客預約同一時段
    let staff_book = call(&s.app, Method::POST, "/t/shop-a/bookings", Some(json!({
        "service_id": s.service_id, "start": start.to_rfc3339(), "customer": {"name": "電話客", "email": "phone@example.com"},
    })), Some(&s.owner)).await;
    assert_eq!(staff_book.0, StatusCode::CREATED);

    // 超過 24 小時才確認:連結失效;worker 把它整理成 cancelled
    sqlx::query(
        "UPDATE bookings SET created_at = now() - interval '25 hours' WHERE status = 'pending'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        call(
            &s.app,
            Method::POST,
            &format!("/public/bookings/{token}/confirm"),
            None,
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let stats = tick(&pool, &mailer, "http://app.test").await.unwrap();
    assert_eq!(stats.expired, 1);
    assert_eq!(
        status_of(&pool, "late@example.com").await,
        vec!["cancelled"]
    );
}

#[sqlx::test]
async fn pending_requests_count_toward_the_customer_limit(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    for h in 9..14 {
        assert_eq!(
            request(&s, at(d, h, 0), "spam@example.com").await.0,
            StatusCode::CREATED
        );
    }
    assert_eq!(
        request(&s, at(d, 15, 0), "spam@example.com").await.0,
        StatusCode::CONFLICT,
        "待確認也算,不能無限灌信"
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM email_outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 5, "被擋下的請求不會多寄信");
}

#[sqlx::test]
async fn staff_created_booking_is_confirmed_immediately(pool: PgPool) {
    let s = shop(&pool).await;
    let (status, created) = call(
        &s.app,
        Method::POST,
        "/t/shop-a/bookings",
        Some(json!({
            "service_id": s.service_id, "start": at(day(3), 10, 0).to_rfc3339(),
            "customer": {"name": "電話客", "email": "phone@example.com"},
        })),
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(created["manage_token"].is_string());
    assert_eq!(
        status_of(&pool, "phone@example.com").await,
        vec!["confirmed"]
    );
    let keys: Vec<String> = sqlx::query_scalar("SELECT dedupe_key FROM email_outbox")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        keys[0].starts_with("confirmed:") && keys.len() == 1,
        "員工代客不需驗證信,只寄確認信: {keys:?}"
    );
}

// ---------- 提醒 ----------

#[sqlx::test]
async fn reminders_are_sent_once_for_the_right_bookings(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let customer: Uuid = sqlx::query_scalar("INSERT INTO customers (tenant_id, name, email) VALUES ($1, '顧客', 'c@example.com') RETURNING id")
        .bind(s.tenant_id).fetch_one(&pool).await.unwrap();
    let service: Uuid = s.service_id.parse().unwrap();

    // (起點距現在, 確認於多久以前, 狀態)
    let cases = [
        (
            "應提醒",
            Duration::hours(20),
            Duration::days(3),
            "confirmed",
        ),
        (
            "剛訂的,剛收過確認信",
            Duration::hours(22),
            Duration::hours(1),
            "confirmed",
        ),
        ("已取消", Duration::hours(5), Duration::days(3), "cancelled"),
        ("還太久", Duration::days(3), Duration::days(5), "confirmed"),
        (
            "已經過去",
            Duration::hours(-5),
            Duration::days(3),
            "confirmed",
        ),
    ];
    for (i, (_, from_now, confirmed_ago, status)) in cases.iter().enumerate() {
        let start = Utc::now() + *from_now;
        sqlx::query("INSERT INTO bookings (tenant_id, staff_user_id, service_id, customer_id, starts_at, ends_at, status, manage_token_hash, confirmed_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7::booking_status, $8, $9)")
            .bind(s.tenant_id).bind(s.owner_id).bind(service).bind(customer)
            .bind(start).bind(start + Duration::minutes(30)).bind(status)
            .bind(format!("hash-{i}")).bind(Utc::now() - *confirmed_ago)
            .execute(&pool).await.unwrap();
    }

    let stats = tick(&pool, &mailer, "http://app.test").await.unwrap();
    assert_eq!((stats.reminders, stats.sent), (1, 1));
    let mails = memory.sent();
    assert!(mails[0].subject.contains("提醒"));
    assert!(
        mails[0].body.contains("http://app.test/bookings/"),
        "提醒信要有改期 / 取消連結: {}",
        mails[0].body
    );

    // 再跑一次不會重複提醒
    assert_eq!(
        tick(&pool, &mailer, "http://app.test").await.unwrap(),
        TickStats::default()
    );
    assert_eq!(memory.sent().len(), 1);
}

// ---------- 寄送、重試、清理 ----------

async fn queue_one_mail(s: &Shop, email: &str) {
    let (status, _) = call(
        &s.app,
        Method::POST,
        "/t/shop-a/bookings",
        Some(json!({
            "service_id": s.service_id, "start": at(day(3), 10, 0).to_rfc3339(),
            "customer": {"name": "顧客", "email": email},
        })),
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

async fn make_due(pool: &PgPool) {
    sqlx::query("UPDATE email_outbox SET run_at = now() WHERE status = 'pending'")
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test]
async fn failed_sends_back_off_and_then_succeed(pool: PgPool) {
    let s = shop(&pool).await;
    queue_one_mail(&s, "c@example.com").await;
    let memory = MemoryMailer::failing(2);
    let mailer = Mailer::Memory(memory.clone());

    let stats = tick(&pool, &mailer, "http://app.test").await.unwrap();
    assert_eq!((stats.sent, stats.retried), (0, 1));
    let (attempts, delay, error): (i32, f64, Option<String>) = sqlx::query_as(
        "SELECT attempts, extract(epoch FROM run_at - now())::float8, last_error FROM email_outbox",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(attempts, 1);
    assert!(delay > 20.0 && delay <= 30.0, "第一次退避約 30 秒: {delay}");
    assert!(error.is_some());

    // 還沒到時間,不會再試
    assert_eq!(
        tick(&pool, &mailer, "http://app.test").await.unwrap(),
        TickStats::default()
    );

    make_due(&pool).await;
    assert_eq!(
        tick(&pool, &mailer, "http://app.test")
            .await
            .unwrap()
            .retried,
        1
    );
    let delay: f64 =
        sqlx::query_scalar("SELECT extract(epoch FROM run_at - now())::float8 FROM email_outbox")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        delay > 50.0 && delay <= 60.0,
        "第二次退避約 60 秒(指數): {delay}"
    );

    make_due(&pool).await;
    assert_eq!(
        tick(&pool, &mailer, "http://app.test").await.unwrap().sent,
        1
    );
    assert_eq!(memory.sent().len(), 1);
    let (status, attempts, body): (String, i32, String) =
        sqlx::query_as("SELECT status, attempts, body FROM email_outbox")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((status.as_str(), attempts, body.as_str()), ("sent", 3, ""));
}

#[sqlx::test]
async fn gives_up_after_max_attempts_and_scrubs_the_body(pool: PgPool) {
    let s = shop(&pool).await;
    queue_one_mail(&s, "c@example.com").await;
    let mailer = Mailer::Memory(MemoryMailer::failing(100));

    let mut failed = 0;
    for _ in 0..worker::MAX_ATTEMPTS {
        make_due(&pool).await;
        failed += tick(&pool, &mailer, "http://app.test")
            .await
            .unwrap()
            .failed;
    }
    assert_eq!(failed, 1);
    let (status, body, error): (String, String, Option<String>) =
        sqlx::query_as("SELECT status, body, last_error FROM email_outbox")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((status.as_str(), body.as_str()), ("failed", ""));
    assert!(error.is_some());
    make_due(&pool).await;
    assert_eq!(
        tick(&pool, &mailer, "http://app.test").await.unwrap(),
        TickStats::default(),
        "failed 不會再被撈出來"
    );
}

#[sqlx::test]
async fn old_finished_mail_is_cleaned_up(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, _) = mailer();
    for (status, age) in [
        ("sent", "40 days"),
        ("failed", "40 days"),
        ("sent", "1 day"),
        ("pending", "40 days"),
    ] {
        sqlx::query("INSERT INTO email_outbox (tenant_id, to_email, subject, body, status, created_at, run_at)
                     VALUES ($1, 'x@example.com', 's', '', $2, now() - $3::interval, now() + interval '1 day')")
            .bind(s.tenant_id).bind(status).bind(age).execute(&pool).await.unwrap();
    }
    assert_eq!(
        tick(&pool, &mailer, "http://app.test")
            .await
            .unwrap()
            .cleaned,
        2
    );
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM email_outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 2, "近期的與仍待寄的都保留");
}

#[sqlx::test]
async fn invitation_emails_are_queued_and_scrubbed_after_sending(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let (status, inv) = call(
        &s.app,
        Method::POST,
        "/t/shop-a/invitations",
        Some(json!({"email": "new@example.com", "role": "staff"})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let token = inv["token"].as_str().unwrap();

    tick(&pool, &mailer, "http://app.test").await.unwrap();
    let mails = memory.sent();
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to, "new@example.com");
    assert!(
        mails[0]
            .body
            .contains(&format!("/invitations/accept#token={token}"))
    );
    let kept: String = sqlx::query_scalar("SELECT body FROM email_outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(kept, "", "寄出後資料庫不留 token");
}

// ---------- 權限與隔離 ----------

#[sqlx::test]
async fn outbox_is_tenant_scoped_and_roles_are_least_privilege(pool: PgPool) {
    let s = shop(&pool).await;
    // 第二家店也排一封信
    let other = signup(&s.app, "z@example.com").await;
    create_tenant(&s.app, &other, "shop-z").await;
    call(
        &s.app,
        Method::POST,
        "/t/shop-z/invitations",
        Some(json!({"email": "n@example.com", "role": "staff"})),
        Some(&other),
    )
    .await;
    call(
        &s.app,
        Method::POST,
        "/t/shop-a/invitations",
        Some(json!({"email": "m@example.com", "role": "staff"})),
        Some(&s.owner),
    )
    .await;
    let tenant_z: Uuid = sqlx::query_scalar("SELECT id FROM tenants WHERE slug = 'shop-z'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let z_owner = user_id(&pool, "z@example.com").await;

    // 請求內的角色:只看得到自己店的信,不能改也不能刪
    let mut tx = begin_scoped(&pool, Some(s.owner_id), Some(s.tenant_id))
        .await
        .unwrap();
    let to: Vec<String> = sqlx::query_scalar("SELECT to_email FROM email_outbox")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(to, vec!["m@example.com"]);
    for sql in [
        "UPDATE email_outbox SET to_email = 'evil@example.com'",
        "DELETE FROM email_outbox",
    ] {
        let err = sqlx::query(sql).execute(&mut *tx).await.unwrap_err();
        assert_eq!(
            err.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501"),
            "{sql}"
        );
        tx = begin_scoped(&pool, Some(s.owner_id), Some(s.tenant_id))
            .await
            .unwrap();
    }
    drop(tx);
    let mut tx = begin_scoped(&pool, Some(z_owner), Some(tenant_z))
        .await
        .unwrap();
    let to: Vec<String> = sqlx::query_scalar("SELECT to_email FROM email_outbox")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(to, vec!["n@example.com"]);
    drop(tx);

    // worker 角色:非超級使用者、無 BYPASSRLS,看得到所有信,但碰不到密碼雜湊,也不能刪預約或寫入服務
    let mut tx = begin_worker(&pool).await.unwrap();
    let (role, bypass, superuser): (String, bool, bool) = sqlx::query_as(
        "SELECT current_user::text, rolbypassrls, rolsuper FROM pg_roles WHERE rolname = current_user").fetch_one(&mut *tx).await.unwrap();
    assert_eq!(role, "tenant_worker");
    assert!(!bypass && !superuser);
    let all: i64 = sqlx::query_scalar("SELECT count(*) FROM email_outbox")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(all, 2);
    drop(tx);
    for sql in [
        "SELECT password_hash FROM users",
        "DELETE FROM bookings",
        "INSERT INTO services (tenant_id, name, duration_minutes) VALUES (gen_random_uuid(), 'x', 30)",
        "SELECT * FROM memberships",
    ] {
        let mut tx = begin_worker(&pool).await.unwrap();
        let err = sqlx::query(sql).execute(&mut *tx).await.unwrap_err();
        assert_eq!(
            err.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501"),
            "{sql}"
        );
    }
}

#[sqlx::test]
async fn background_loop_delivers_mail_and_stops_on_shutdown(pool: PgPool) {
    let s = shop(&pool).await;
    let memory = MemoryMailer::new();
    let (tx, rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(worker::run(
        pool.clone(),
        Mailer::Memory(memory.clone()),
        "http://app.test".to_string(),
        StdDuration::from_millis(50),
        Some(730),
        rx,
    ));

    queue_one_mail(&s, "c@example.com").await;
    let mut delivered = false;
    for _ in 0..60 {
        if !memory.sent().is_empty() {
            delivered = true;
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(50)).await;
    }
    assert!(delivered, "worker 應在幾秒內寄出信");

    tx.send(true).unwrap();
    tokio::time::timeout(StdDuration::from_secs(3), handle)
        .await
        .expect("收到關閉訊號後應該停止")
        .unwrap();
}

#[sqlx::test]
async fn confirmation_rechecks_that_the_slot_is_still_bookable(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let d = day(3);
    let start = at(d, 10, 0);

    request(&s, start, "c@example.com").await;
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    let token = token_in(&memory.sent()[0].body);

    // 顧客還沒確認時,員工新增了涵蓋該時段的休假
    let off = json!({"starts_at": at(d, 9, 0).to_rfc3339(), "ends_at": at(d, 12, 0).to_rfc3339(), "reason": "臨時有事"});
    call(
        &s.app,
        Method::POST,
        &format!("/t/shop-a/members/{}/time-off", s.owner_id),
        Some(off),
        Some(&s.owner),
    )
    .await;

    let (status, _) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/confirm"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(status_of(&pool, "c@example.com").await, vec!["cancelled"]);
}

#[sqlx::test]
async fn one_failing_stage_does_not_stop_mail_delivery(pool: PgPool) {
    let s = shop(&pool).await;
    queue_one_mail(&s, "c@example.com").await;
    let (mailer, memory) = mailer();
    // 讓「排入提醒」階段出錯:租戶的時區變成程式認不得的值
    let customer: Uuid = sqlx::query_scalar("INSERT INTO customers (tenant_id, name, email) VALUES ($1, 'x', 'r@example.com') RETURNING id")
        .bind(s.tenant_id).fetch_one(&pool).await.unwrap();
    let start = Utc::now() + Duration::hours(20);
    sqlx::query("INSERT INTO bookings (tenant_id, staff_user_id, service_id, customer_id, starts_at, ends_at, status, manage_token_hash, confirmed_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'confirmed', 'h', now() - interval '3 days')")
        .bind(s.tenant_id).bind(s.owner_id).bind(s.service_id.parse::<Uuid>().unwrap()).bind(customer)
        .bind(start).bind(start + Duration::hours(1)).execute(&pool).await.unwrap();
    sqlx::query("UPDATE tenants SET timezone = 'Not/AZone'")
        .execute(&pool)
        .await
        .unwrap();

    let stats = tick(&pool, &mailer, "http://app.test").await.unwrap();
    assert_eq!(stats.reminders, 0);
    assert_eq!(stats.sent, 1, "提醒出錯,但待寄的信照樣寄出");
    assert_eq!(memory.sent().len(), 1);
}

/// Log 模式會把含一次性連結的信件內容印進日誌,只能用在開發;正式環境沒設 SMTP 就必須拒絕
#[test]
fn production_refuses_to_log_emails_instead_of_sending_them() {
    use tenant_saas::config::Config;
    let base = common::test_config(3600);

    let dev = Config {
        production: false,
        smtp_url: None,
        ..base.clone()
    };
    assert!(
        matches!(Mailer::from_config(&dev), Ok(Mailer::Log)),
        "開發環境沒設 SMTP 可以用 Log 模式"
    );

    let prod = Config {
        production: true,
        smtp_url: None,
        ..base.clone()
    };
    let err = Mailer::from_config(&prod)
        .err()
        .expect("正式環境必須拒絕")
        .to_string();
    assert!(err.contains("SMTP_URL"), "{err}");

    let prod_smtp = Config {
        production: true,
        smtp_url: Some("smtp://127.0.0.1:1025".into()),
        ..base
    };
    assert!(matches!(
        Mailer::from_config(&prod_smtp),
        Ok(Mailer::Smtp { .. })
    ));
}

/// 改期之後要針對「新的時間」重新提醒。原本 `reminder_queued_at` 不會清,
/// 去重鍵又只有預約 id,所以改期後永遠不會再收到提醒。
#[sqlx::test]
async fn rescheduled_bookings_get_a_fresh_reminder(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let customer: Uuid = sqlx::query_scalar("INSERT INTO customers (tenant_id, name, email) VALUES ($1, '顧客', 'c@example.com') RETURNING id")
        .bind(s.tenant_id).fetch_one(&pool).await.unwrap();
    let raw = "r".repeat(64);
    let start = Utc::now() + Duration::hours(20);
    sqlx::query("INSERT INTO bookings (tenant_id, staff_user_id, service_id, customer_id, starts_at, ends_at, status, manage_token_hash, confirmed_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'confirmed', $7, now() - interval '3 days')")
        .bind(s.tenant_id).bind(s.owner_id).bind(s.service_id.parse::<Uuid>().unwrap()).bind(customer)
        .bind(start).bind(start + Duration::hours(1)).bind(tenant_saas::token::hash(&raw))
        .execute(&pool).await.unwrap();

    // 第一次提醒
    assert_eq!(
        tick(&pool, &mailer, "http://app.test")
            .await
            .unwrap()
            .reminders,
        1
    );

    // 顧客改到三天後(合法的營業時段)
    let new_start = at(day(3), 10, 0);
    let (status, body) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{raw}/reschedule"),
        Some(json!({"start": new_start.to_rfc3339()})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let queued: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT reminder_queued_at FROM bookings")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(queued.is_none(), "改期後必須重新排提醒");

    // 模擬時間來到新預約的前一天:應該收到針對新時間的第二封提醒
    sqlx::query("UPDATE bookings SET starts_at = now() + interval '20 hours', ends_at = now() + interval '21 hours'").execute(&pool).await.unwrap();
    assert_eq!(
        tick(&pool, &mailer, "http://app.test")
            .await
            .unwrap()
            .reminders,
        1
    );
    let subjects: Vec<String> = memory.sent().iter().map(|m| m.subject.clone()).collect();
    assert_eq!(
        subjects.iter().filter(|x| x.contains("提醒")).count(),
        2,
        "寄出的信:{subjects:?}"
    );
}

/// A → B → 再改回 A:每一輪都要提醒(去重鍵若用開始時間,第三輪會與第一輪撞鍵而被吞掉)
#[sqlx::test]
async fn reminders_are_sent_again_after_moving_back_to_the_original_time(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let customer: Uuid = sqlx::query_scalar("INSERT INTO customers (tenant_id, name, email) VALUES ($1, '顧客', 'c@example.com') RETURNING id")
        .bind(s.tenant_id).fetch_one(&pool).await.unwrap();
    let raw = "q".repeat(64);
    let a = at(day(3), 10, 0);
    let b = at(day(3), 15, 0);
    sqlx::query("INSERT INTO bookings (tenant_id, staff_user_id, service_id, customer_id, starts_at, ends_at, status, manage_token_hash, confirmed_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'confirmed', $7, now() - interval '5 days')")
        .bind(s.tenant_id).bind(s.owner_id).bind(s.service_id.parse::<Uuid>().unwrap()).bind(customer)
        .bind(a).bind(a + Duration::hours(1)).bind(tenant_saas::token::hash(&raw))
        .execute(&pool).await.unwrap();

    for target in [b, a, b] {
        // 模擬「提醒窗口到了」:把開始時間拉到 20 小時後,寄出提醒,再改期到 target
        sqlx::query("UPDATE bookings SET starts_at = now() + interval '20 hours', ends_at = now() + interval '21 hours'").execute(&pool).await.unwrap();
        assert_eq!(
            tick(&pool, &mailer, "http://app.test")
                .await
                .unwrap()
                .reminders,
            1
        );
        let (status, body) = call(
            &s.app,
            Method::POST,
            &format!("/public/bookings/{raw}/reschedule"),
            Some(json!({"start": target.to_rfc3339()})),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(
        memory
            .sent()
            .iter()
            .filter(|m| m.subject.contains("提醒"))
            .count(),
        3
    );
}

// ---------- 重寄確認信 ----------

async fn resend(s: &Shop, id: &str, email: &str) -> (StatusCode, Value) {
    call(
        &s.app,
        Method::POST,
        &format!("/public/shops/shop-a/bookings/{id}/resend"),
        Some(json!({"email": email})),
        None,
    )
    .await
}

/// 讓「上次寄出」的時間往前推,模擬等過了最短間隔
async fn age_last_send(pool: &PgPool) {
    sqlx::query("UPDATE bookings SET verification_sent_at = now() - interval '2 minutes'")
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test]
async fn resending_issues_a_new_link_and_voids_the_old_one(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let (_, created) = request(&s, at(day(3), 10, 0), "c@example.com").await;
    let id = created["id"].as_str().unwrap();
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    let old = token_in(&memory.sent()[0].body);

    age_last_send(&pool).await;
    // Email 大小寫與前後空白不影響
    let (status, _) = resend(&s, id, "  C@Example.com ").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    let mails = memory.sent();
    assert_eq!(mails.len(), 2);
    assert_eq!(mails[1].to, "c@example.com");
    let new = token_in(&mails[1].body);
    assert_ne!(new, old);

    // 舊連結失效,新連結可以確認
    assert_eq!(
        call(
            &s.app,
            Method::GET,
            &format!("/public/bookings/{old}"),
            None,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &s.app,
            Method::POST,
            &format!("/public/bookings/{new}/confirm"),
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action = 'booking.verification_resent'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test]
async fn resend_never_reveals_whether_a_booking_exists(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let (_, created) = request(&s, at(day(3), 10, 0), "c@example.com").await;
    let id = created["id"].as_str().unwrap();
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    age_last_send(&pool).await;

    let real = resend(&s, id, "someone-else@example.com").await; // 編號對、Email 不符
    let unknown = resend(&s, &uuid::Uuid::new_v4().to_string(), "c@example.com").await; // 編號不存在
    assert_eq!(real.0, StatusCode::ACCEPTED);
    assert_eq!(real, unknown, "回應必須一模一樣");
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    assert_eq!(
        memory.sent().len(),
        1,
        "對不上的請求不能寄信,也不能換掉 token"
    );

    // 別家店的路徑也一樣(RLS 下看不到)
    let (status, _) = call(
        &s.app,
        Method::POST,
        &format!("/public/shops/no-such-shop/bookings/{id}/resend"),
        Some(json!({"email": "c@example.com"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn resend_is_limited_in_count_and_frequency(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let (_, created) = request(&s, at(day(3), 10, 0), "c@example.com").await;
    let id = created["id"].as_str().unwrap();

    // 剛寄出不到一分鐘:不重寄
    resend(&s, id, "c@example.com").await;
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    assert_eq!(memory.sent().len(), 1, "只有最初的那封");

    for _ in 0..6 {
        age_last_send(&pool).await;
        resend(&s, id, "c@example.com").await;
    }
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    assert_eq!(memory.sent().len(), 1 + 3, "最多重寄 3 次");
}

#[sqlx::test]
async fn resend_only_works_while_the_request_is_still_pending(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let (_, created) = request(&s, at(day(3), 10, 0), "c@example.com").await;
    let id = created["id"].as_str().unwrap();
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    let token = token_in(&memory.sent()[0].body);
    age_last_send(&pool).await;

    // 已過 24 小時
    sqlx::query("UPDATE bookings SET created_at = now() - interval '25 hours'")
        .execute(&pool)
        .await
        .unwrap();
    resend(&s, id, "c@example.com").await;
    sqlx::query("UPDATE bookings SET created_at = now()")
        .execute(&pool)
        .await
        .unwrap();

    // 已確認
    call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/confirm"),
        None,
        None,
    )
    .await;
    resend(&s, id, "c@example.com").await;

    tick(&pool, &mailer, "http://app.test").await.unwrap();
    let verifications = memory
        .sent()
        .iter()
        .filter(|m| m.subject.contains("請確認"))
        .count();
    assert_eq!(
        verifications, 1,
        "過期或已確認的不再寄驗證信,也不會換掉管理連結"
    );
    assert_eq!(
        call(
            &s.app,
            Method::GET,
            &format!("/public/bookings/{token}"),
            None,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}

/// 申請(待確認)之後,負責的員工離職被停用 → 顧客再來確認時要失敗,
/// 而不是悄悄把預約確認在一位已經不在的員工身上
#[sqlx::test]
async fn confirming_after_the_staff_was_deactivated_fails_cleanly(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    // b 是唯一提供服務的人:owner 不再提供
    signup(&s.app, "b@example.com").await;
    add_member(&pool, "shop-a", "b@example.com", "staff").await;
    let b = user_id(&pool, "b@example.com").await;
    let hours: Vec<Value> = (0..7)
        .map(|d| json!({"weekday": d, "start": "09:00", "end": "18:00"}))
        .collect();
    for (who, services) in [(b, vec![s.service_id.clone()]), (s.owner_id, vec![])] {
        call(
            &s.app,
            Method::PUT,
            &format!("/t/shop-a/members/{who}/working-hours"),
            Some(json!({"hours": hours})),
            Some(&s.owner),
        )
        .await;
        call(
            &s.app,
            Method::PUT,
            &format!("/t/shop-a/members/{who}/services"),
            Some(json!({"service_ids": services})),
            Some(&s.owner),
        )
        .await;
    }

    let (status, _) = request(&s, at(day(3), 10, 0), "c@example.com").await;
    assert_eq!(status, StatusCode::CREATED);
    let assigned: Uuid = sqlx::query_scalar("SELECT staff_user_id FROM bookings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(assigned, b);
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    let token = token_in(&memory.sent()[0].body);

    // 待確認的預約不擋停用
    let (status, _) = call(
        &s.app,
        Method::POST,
        &format!("/t/shop-a/members/{b}/deactivate"),
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/confirm"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(status_of(&pool, "c@example.com").await, vec!["cancelled"]);
    let reason: String = sqlx::query_scalar(
        "SELECT detail->>'reason' FROM audit_logs WHERE action = 'booking.cancelled'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(reason, "no_longer_bookable");
}

/// 提醒信的連結:專用的 token(資料庫只存雜湊),能管理預約,而且不影響確認信的連結
#[sqlx::test]
async fn reminder_link_manages_the_booking_without_replacing_the_confirmation_link(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let confirmation = confirmed_booking_in_20h(&s, &pool).await;

    assert_eq!(
        tick(&pool, &mailer, "http://app.test")
            .await
            .unwrap()
            .reminders,
        1
    );
    let reminder_token = token_in(&memory.sent()[0].body);
    assert_eq!(reminder_token.len(), 64);
    assert_ne!(reminder_token, confirmation, "兩封信的連結是不同的祕密");

    // 資料庫只存雜湊:原始 token 不在任何欄位,寄出後信件內容也被清掉
    let (hash, body): (String, String) = sqlx::query_as(
        "SELECT b.reminder_token_hashes[1], (SELECT body FROM email_outbox LIMIT 1) FROM bookings b",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(hash, tenant_saas::token::hash(&reminder_token));
    assert_ne!(hash, reminder_token);
    assert_eq!(body, "");

    // 提醒信的連結看得到預約,也查得到改期要用的可預約時段
    let get = |t: String| {
        let app = s.app.clone();
        async move {
            call(
                &app,
                Method::GET,
                &format!("/public/bookings/{t}"),
                None,
                None,
            )
            .await
        }
    };
    let (status, view) = get(reminder_token.clone()).await;
    assert_eq!(
        (status, view["status"].as_str()),
        (StatusCode::OK, Some("confirmed"))
    );
    let (status, _) = call(
        &s.app,
        Method::GET,
        &format!(
            "/public/bookings/{reminder_token}/availability?from={}",
            day(2)
        ),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // 確認信的連結沒有因此失效;亂猜的仍是 404
    assert_eq!(get(confirmation.clone()).await.0, StatusCode::OK);
    assert_eq!(get("0".repeat(64)).await.0, StatusCode::NOT_FOUND);

    // 對已確認的預約再按「確認」(例如重複點擊)是無害的成功,不會改變任何東西
    let (status, view) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{reminder_token}/confirm"),
        None,
        None,
    )
    .await;
    assert_eq!(
        (status, view["status"].as_str()),
        (StatusCode::OK, Some("confirmed"))
    );

    // 用提醒信的連結取消 → 成功,兩個連結看到的是同一筆已取消的預約
    let (status, view) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{reminder_token}/cancel"),
        None,
        None,
    )
    .await;
    assert_eq!(
        (status, view["status"].as_str()),
        (StatusCode::OK, Some("cancelled"))
    );
    assert_eq!(get(confirmation).await.1["status"], "cancelled");
}

/// 提醒信的連結能改期;改期後重新提醒時發新的 token,上一封提醒信的連結**仍然有效**(確認信的連結一直有效,
/// 讓舊提醒連結失效沒有換到任何安全性,只會讓顧客點舊信時看到「找不到」)
#[sqlx::test]
async fn a_new_reminder_after_rescheduling_keeps_the_old_reminder_link_working(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let confirmation = confirmed_booking_in_20h(&s, &pool).await;
    assert_eq!(
        tick(&pool, &mailer, "http://app.test")
            .await
            .unwrap()
            .reminders,
        1
    );
    let first = token_in(&memory.sent()[0].body);

    // 用第一封提醒信的連結改期
    let (status, body) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{first}/reschedule"),
        Some(json!({"start": at(day(3), 10, 0).to_rfc3339()})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // 新時間的前一天:第二封提醒,連結是新的
    sqlx::query("UPDATE bookings SET starts_at = now() + interval '20 hours', ends_at = now() + interval '21 hours'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        tick(&pool, &mailer, "http://app.test")
            .await
            .unwrap()
            .reminders,
        1
    );
    let mails = memory.sent();
    let second = token_in(
        &mails
            .iter()
            .rfind(|m| m.subject.contains("提醒"))
            .unwrap()
            .body,
    );
    assert_ne!(first, second);

    let get = |t: &str| {
        let (app, t) = (s.app.clone(), t.to_string());
        async move {
            call(
                &app,
                Method::GET,
                &format!("/public/bookings/{t}"),
                None,
                None,
            )
            .await
            .0
        }
    };
    assert_eq!(get(&second).await, StatusCode::OK);
    assert_eq!(
        get(&first).await,
        StatusCode::OK,
        "上一封提醒信的連結仍然有效"
    );
    assert_eq!(
        get(&confirmation).await,
        StatusCode::OK,
        "確認信的連結一直有效"
    );
}

/// 提醒連結最多保留最近 10 個:超過時丟掉最舊的,新的接在最後(不會無限成長)
#[sqlx::test]
async fn only_the_ten_most_recent_reminder_links_are_kept(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    confirmed_booking_in_20h(&s, &pool).await;
    sqlx::query(
        "UPDATE bookings SET reminder_token_hashes =
            ARRAY['h1','h2','h3','h4','h5','h6','h7','h8','h9','h10'], reminder_queued_at = NULL",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        tick(&pool, &mailer, "http://app.test")
            .await
            .unwrap()
            .reminders,
        1
    );
    let newest = token_in(&memory.sent()[0].body);
    let hashes: Vec<String> =
        sqlx::query_scalar("SELECT unnest(reminder_token_hashes) FROM bookings")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(hashes.len(), 10);
    assert_eq!(hashes[0], "h2", "最舊的 h1 被丟掉");
    assert_eq!(hashes[9], tenant_saas::token::hash(&newest));
}

/// 20 小時後開始、3 天前確認的預約(符合提醒條件)。回傳確認信連結的原始 token
async fn confirmed_booking_in_20h(s: &Shop, pool: &PgPool) -> String {
    let customer: Uuid = sqlx::query_scalar("INSERT INTO customers (tenant_id, name, email) VALUES ($1, '顧客', 'c@example.com') RETURNING id")
        .bind(s.tenant_id).fetch_one(pool).await.unwrap();
    let confirmation = "k".repeat(64);
    let start = Utc::now() + Duration::hours(20);
    sqlx::query("INSERT INTO bookings (tenant_id, staff_user_id, service_id, customer_id, starts_at, ends_at, status, manage_token_hash, confirmed_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'confirmed', $7, now() - interval '3 days')")
        .bind(s.tenant_id).bind(s.owner_id).bind(s.service_id.parse::<Uuid>().unwrap()).bind(customer)
        .bind(start).bind(start + Duration::hours(1)).bind(tenant_saas::token::hash(&confirmation))
        .execute(pool).await.unwrap();
    confirmation
}
