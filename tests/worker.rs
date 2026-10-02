mod common;

use std::time::Duration as StdDuration;

use axum::{
    Router,
    http::{Method, StatusCode},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Asia::Taipei;
use common::{app, call, create_service, create_tenant, signup, user_id};
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
    let stats = tick(&pool, &mailer).await.unwrap();
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
    tick(&pool, &mailer).await.unwrap();
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
    tick(&pool, &mailer).await.unwrap();
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
    tick(&pool, &mailer).await.unwrap();
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
    let stats = tick(&pool, &mailer).await.unwrap();
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

    let stats = tick(&pool, &mailer).await.unwrap();
    assert_eq!((stats.reminders, stats.sent), (1, 1));
    let mails = memory.sent();
    assert!(mails[0].subject.contains("提醒"));
    assert!(
        !mails[0].body.contains("/bookings/"),
        "提醒信不含管理連結(資料庫只存雜湊)"
    );

    // 再跑一次不會重複提醒
    assert_eq!(tick(&pool, &mailer).await.unwrap(), TickStats::default());
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

    let stats = tick(&pool, &mailer).await.unwrap();
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
    assert_eq!(tick(&pool, &mailer).await.unwrap(), TickStats::default());

    make_due(&pool).await;
    assert_eq!(tick(&pool, &mailer).await.unwrap().retried, 1);
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
    assert_eq!(tick(&pool, &mailer).await.unwrap().sent, 1);
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
        failed += tick(&pool, &mailer).await.unwrap().failed;
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
        tick(&pool, &mailer).await.unwrap(),
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
    assert_eq!(tick(&pool, &mailer).await.unwrap().cleaned, 2);
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

    tick(&pool, &mailer).await.unwrap();
    let mails = memory.sent();
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to, "new@example.com");
    assert!(mails[0].body.contains(token) && mails[0].body.contains("/invitations/accept?token="));
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
        StdDuration::from_millis(50),
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
    tick(&pool, &mailer).await.unwrap();
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

    let stats = tick(&pool, &mailer).await.unwrap();
    assert_eq!(stats.reminders, 0);
    assert_eq!(stats.sent, 1, "提醒出錯,但待寄的信照樣寄出");
    assert_eq!(memory.sent().len(), 1);
}
