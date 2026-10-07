//! 服務的整理時間(緩衝):結束後員工多久不能接下一筆。

use crate::common::{add_member, app, call, create_tenant, signup, user_id};
use axum::{Router, http::Method, http::StatusCode};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Asia::Taipei;
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

fn hhmm(t: DateTime<Utc>) -> String {
    t.with_timezone(&Taipei).format("%H:%M").to_string()
}

struct Shop {
    app: Router,
    owner: String,
    owner_id: Uuid,
    tenant_id: Uuid,
    /// 30 分鐘,整理 15 分鐘
    cut: String,
    /// 30 分鐘,沒有整理時間
    quick: String,
}

async fn make_service(app: &Router, owner: &str, name: &str, buffer: i32) -> String {
    let (status, body) = call(
        app,
        Method::POST,
        "/t/shop-a/services",
        Some(json!({"name": name, "duration_minutes": 30, "buffer_minutes": buffer, "price_cents": 100})),
        Some(owner),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["buffer_minutes"], buffer);
    body["id"].as_str().unwrap().to_string()
}

/// owner 每天 09:00–18:00 提供兩項服務:「剪髮」(整理 15 分鐘)與「快剪」(沒有整理時間)
async fn shop(pool: &PgPool) -> Shop {
    let app = app(pool.clone());
    let owner = signup(&app, "o@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    let cut = make_service(&app, &owner, "剪髮", 15).await;
    let quick = make_service(&app, &owner, "快剪", 0).await;
    let owner_id = user_id(pool, "o@example.com").await;
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
        Some(json!({"service_ids": [cut, quick]})),
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
        tenant_id,
        cut,
        quick,
    }
}

/// 店主代客預約。回傳 (狀態, 回應)
async fn book(
    s: &Shop,
    service: &str,
    d: NaiveDate,
    h: u32,
    m: u32,
    email: &str,
) -> (StatusCode, Value) {
    call(
        &s.app,
        Method::POST,
        "/t/shop-a/bookings",
        Some(
            json!({"service_id": service, "start": at(d, h, m).to_rfc3339(),
            "customer": {"name": "顧客", "email": email}}),
        ),
        Some(&s.owner),
    )
    .await
}

/// 公開預約頁看到的那天的可預約起點("HH:MM")
async fn slots(s: &Shop, service: &str, d: NaiveDate) -> Vec<String> {
    let (status, body) = call(
        &s.app,
        Method::GET,
        &format!("/public/shops/shop-a/availability?service_id={service}&from={d}"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["slots"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| {
            let start: DateTime<Utc> = x["start"].as_str().unwrap().parse().unwrap();
            (start.with_timezone(&Taipei).date_naive() == d).then(|| hhmm(start))
        })
        .collect()
}

async fn blocked_until(pool: &PgPool, id: &str) -> DateTime<Utc> {
    sqlx::query_scalar("SELECT blocked_until FROM bookings WHERE id = $1::uuid")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn the_buffer_pushes_the_next_booking_later_and_pulls_the_previous_one_earlier(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    // 剪髮 10:00–10:30,整理到 10:45
    let (status, created) = book(&s, &s.cut, d, 10, 0, "a@example.com").await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    // 顧客看到的結束時間不含整理時間;員工被占住到 10:45
    assert_eq!(
        hhmm(created["ends_at"].as_str().unwrap().parse().unwrap()),
        "10:30"
    );
    assert_eq!(
        hhmm(blocked_until(&pool, created["id"].as_str().unwrap()).await),
        "10:45"
    );

    // 另一個「剪髮」(自己也要整理 15 分鐘):
    let cut = slots(&s, &s.cut, d).await;
    assert!(
        !cut.contains(&"10:30".to_string()),
        "10:30 還在整理: {cut:?}"
    );
    assert!(cut.contains(&"10:45".to_string()));
    assert!(
        !cut.contains(&"09:30".to_string()),
        "09:30–10:00 結束後整理到 10:15,會撞到 10:00 的預約"
    );
    assert!(
        cut.contains(&"09:15".to_string()),
        "09:15 結束 09:45,整理到 10:00,剛好接上"
    );
    // 「快剪」沒有整理時間:可以緊貼在前面(09:30–10:00),但員工在 10:45 前還在整理
    let quick = slots(&s, &s.quick, d).await;
    assert!(quick.contains(&"09:30".to_string()), "{quick:?}");
    assert!(!quick.contains(&"10:30".to_string()) && quick.contains(&"10:45".to_string()));

    // 實際預約跟可預約時段一致
    assert_eq!(
        book(&s, &s.cut, d, 10, 30, "b@example.com").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        book(&s, &s.quick, d, 10, 30, "b@example.com").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        book(&s, &s.cut, d, 10, 45, "b@example.com").await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        book(&s, &s.quick, d, 9, 30, "c@example.com").await.0,
        StatusCode::CREATED
    );
}

#[sqlx::test]
async fn the_buffer_can_run_past_closing_time_but_the_service_cannot(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    // 營業到 18:00:17:30–18:00 的剪髮可以(整理到 18:15),18:00 開始的不行
    let cut = slots(&s, &s.cut, d).await;
    assert_eq!(cut.last().unwrap(), "17:30");
    let (status, created) = book(&s, &s.cut, d, 17, 30, "a@example.com").await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        hhmm(blocked_until(&pool, created["id"].as_str().unwrap()).await),
        "18:15"
    );
    assert_eq!(
        book(&s, &s.cut, d, 18, 0, "b@example.com").await.0,
        StatusCode::CONFLICT
    );
}

#[sqlx::test]
async fn customer_self_booking_and_confirmation_respect_the_buffer(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    book(&s, &s.cut, d, 10, 0, "a@example.com").await;
    let request = |h: u32, m: u32| {
        let (app, service) = (s.app.clone(), s.cut.clone());
        async move {
            call(
                &app,
                Method::POST,
                "/public/shops/shop-a/bookings",
                Some(
                    json!({"service_id": service, "start": at(d, h, m).to_rfc3339(),
                    "customer": {"name": "客", "email": "self@example.com"}}),
                ),
                None,
            )
            .await
        }
    };
    assert_eq!(
        request(10, 30).await.0,
        StatusCode::CONFLICT,
        "整理中不能申請"
    );
    let (status, requested) = request(10, 45).await;
    assert_eq!(status, StatusCode::CREATED, "{requested}");

    // 待確認不占時段,也沒有整理時間;確認之後才占住,並記下 blocked_until
    let id = requested["id"].as_str().unwrap();
    let blocked_pending: DateTime<Utc> =
        sqlx::query_scalar("SELECT blocked_until FROM bookings WHERE id = $1::uuid")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        hhmm(blocked_pending),
        "11:30",
        "10:45 開始 + 30 分鐘服務 + 15 分鐘整理"
    );
    let token = "t".repeat(64);
    sqlx::query("UPDATE bookings SET manage_token_hash = $2 WHERE id = $1::uuid")
        .bind(id)
        .bind(tenant_saas::token::hash(&token))
        .execute(&pool)
        .await
        .unwrap();
    let (status, view) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/confirm"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    // 確認之後 10:45 這筆真的占住了(到 11:30):緊接著的時段被整理時間擋住
    assert_eq!(
        book(&s, &s.quick, d, 10, 45, "z@example.com").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        book(&s, &s.quick, d, 11, 15, "z@example.com").await.0,
        StatusCode::CONFLICT,
        "整理到 11:30"
    );
    assert_eq!(
        book(&s, &s.quick, d, 11, 30, "z@example.com").await.0,
        StatusCode::CREATED
    );
}

#[sqlx::test]
async fn editing_a_services_buffer_only_affects_new_bookings(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    let (_, old) = book(&s, &s.cut, d, 10, 0, "a@example.com").await;
    let old_id = old["id"].as_str().unwrap();
    assert_eq!(hhmm(blocked_until(&pool, old_id).await), "10:45");

    // 整理時間改成 0:已存在的預約仍然占到 10:45(快照),不會突然變成可以插進去
    let (status, body) = call(
        &s.app,
        Method::PATCH,
        &format!("/t/shop-a/services/{}", s.cut),
        Some(json!({"buffer_minutes": 0})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(hhmm(blocked_until(&pool, old_id).await), "10:45");
    assert_eq!(
        book(&s, &s.cut, d, 10, 30, "b@example.com").await.0,
        StatusCode::CONFLICT
    );

    // 新的預約用新的設定:沒有整理時間
    let (status, fresh) = book(&s, &s.cut, d, 10, 45, "b@example.com").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        hhmm(blocked_until(&pool, fresh["id"].as_str().unwrap()).await),
        "11:15"
    );

    // 改成 30:新預約整理 30 分鐘;稽核有記下改了哪個欄位
    call(
        &s.app,
        Method::PATCH,
        &format!("/t/shop-a/services/{}", s.cut),
        Some(json!({"buffer_minutes": 30})),
        Some(&s.owner),
    )
    .await;
    let (_, later) = book(&s, &s.cut, d, 13, 0, "c@example.com").await;
    assert_eq!(
        hhmm(blocked_until(&pool, later["id"].as_str().unwrap()).await),
        "14:00"
    );
    let changed: Vec<Value> = sqlx::query_scalar(
        "SELECT detail->'changed' FROM audit_logs WHERE action = 'service.updated' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        changed,
        [json!({"buffer_minutes": 0}), json!({"buffer_minutes": 30})]
    );
}

#[sqlx::test]
async fn rescheduling_respects_the_buffer_and_recomputes_it(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    let (_, y) = book(&s, &s.cut, d, 10, 0, "y@example.com").await;
    book(&s, &s.cut, d, 12, 0, "x@example.com").await; // 12:00–12:30,整理到 12:45
    let id = y["id"].as_str().unwrap();
    let resched = |h: u32, m: u32| {
        let (app, owner, id) = (s.app.clone(), s.owner.clone(), id.to_string());
        async move {
            call(
                &app,
                Method::POST,
                &format!("/t/shop-a/bookings/{id}/reschedule"),
                Some(json!({"start": at(d, h, m).to_rfc3339()})),
                Some(&owner),
            )
            .await
            .0
        }
    };
    // 11:45–12:15 撞到;11:30 結束 12:00 但整理到 12:15,也撞到 12:00 的預約;11:15 結束 11:45、整理到 12:00 剛好
    assert_eq!(resched(11, 45).await, StatusCode::CONFLICT);
    assert_eq!(resched(11, 30).await, StatusCode::CONFLICT);
    // 在自己原本的整理時間內小幅移動:排除自己,可以
    assert_eq!(resched(10, 15).await, StatusCode::OK);
    assert_eq!(
        hhmm(blocked_until(&pool, id).await),
        "11:00",
        "10:15 + 30 + 15,不是舊的 10:45"
    );
    assert_eq!(resched(11, 15).await, StatusCode::OK);
    assert_eq!(hhmm(blocked_until(&pool, id).await), "12:00");

    // 顧客自己改期(用管理連結)走同一套規則
    let token = "m".repeat(64);
    sqlx::query("UPDATE bookings SET manage_token_hash = $2 WHERE id = $1::uuid")
        .bind(id)
        .bind(tenant_saas::token::hash(&token))
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/reschedule"),
        Some(json!({"start": at(d, 11, 30).to_rfc3339()})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/reschedule"),
        Some(json!({"start": at(d, 15, 0).to_rfc3339()})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hhmm(blocked_until(&pool, id).await), "15:45");
}

/// 資料庫是最後一道防線:就算應用程式的檢查被繞過(或兩個請求同時進來),也擠不進整理時間
#[sqlx::test]
async fn the_database_itself_refuses_bookings_inside_a_buffer(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    let customer: Uuid = sqlx::query_scalar("INSERT INTO customers (tenant_id, name, email) VALUES ($1, 'c', 'c@example.com') RETURNING id")
        .bind(s.tenant_id).fetch_one(&pool).await.unwrap();
    let service: Uuid = s.cut.parse().unwrap();
    let insert = |h: u32, m: u32, blocked_min: i64, status: &'static str| {
        let (pool, tenant, owner) = (pool.clone(), s.tenant_id, s.owner_id);
        async move {
            let start = at(d, h, m);
            sqlx::query(
                "INSERT INTO bookings (tenant_id, staff_user_id, service_id, customer_id, starts_at, ends_at,
                                       blocked_until, status, manage_token_hash)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8::booking_status, md5(random()::text) || md5(random()::text))",
            )
            .bind(tenant).bind(owner).bind(service).bind(customer)
            .bind(start).bind(start + Duration::minutes(30)).bind(start + Duration::minutes(blocked_min))
            .bind(status)
            .execute(&pool)
            .await
        }
    };
    insert(10, 0, 45, "confirmed").await.unwrap(); // 10:00–10:30,占到 10:45
    let err = insert(10, 30, 45, "confirmed").await.unwrap_err();
    assert_eq!(
        tenant_saas::db::pg_code(&err).as_deref(),
        Some("23P01"),
        "10:30 在整理時間內: {err}"
    );
    insert(10, 45, 75, "confirmed").await.unwrap(); // 10:45 剛好接上
    // 待確認與已取消的不占時段
    insert(10, 30, 45, "pending").await.unwrap();
    insert(10, 30, 45, "cancelled").await.unwrap();
    // 整理時間不能比結束時間短
    let err = insert(12, 0, 15, "confirmed").await.unwrap_err();
    assert_eq!(
        tenant_saas::db::pg_code(&err).as_deref(),
        Some("23514"),
        "{err}"
    );
}

#[sqlx::test]
async fn writes_that_do_not_mention_blocked_until_get_a_safe_default(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    let customer: Uuid = sqlx::query_scalar("INSERT INTO customers (tenant_id, name, email) VALUES ($1, 'c', 'c@example.com') RETURNING id")
        .bind(s.tenant_id).fetch_one(&pool).await.unwrap();
    // 不寫 blocked_until:預設等於 ends_at(沒有整理時間)
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO bookings (tenant_id, staff_user_id, service_id, customer_id, starts_at, ends_at, manage_token_hash)
         VALUES ($1, $2, $3, $4, $5, $6, md5('a') || md5('b')) RETURNING id",
    )
    .bind(s.tenant_id).bind(s.owner_id).bind(s.cut.parse::<Uuid>().unwrap()).bind(customer)
    .bind(at(d, 10, 0)).bind(at(d, 10, 30))
    .fetch_one(&pool).await.unwrap();
    assert_eq!(hhmm(blocked_until(&pool, &id.to_string()).await), "10:30");
    // 只改 ends_at 的更新:整理時間的長度維持不變
    sqlx::query(
        "UPDATE bookings SET blocked_until = ends_at + interval '20 minutes' WHERE id = $1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE bookings SET ends_at = $2 WHERE id = $1")
        .bind(id)
        .bind(at(d, 11, 0))
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(hhmm(blocked_until(&pool, &id.to_string()).await), "11:20");
}

#[sqlx::test]
async fn buffer_validation_defaults_and_permissions(pool: PgPool) {
    let s = shop(&pool).await;
    let create = |buffer: Value| {
        let (app, owner) = (s.app.clone(), s.owner.clone());
        async move {
            call(
                &app,
                Method::POST,
                "/t/shop-a/services",
                Some(json!({"name": "測試", "duration_minutes": 30, "buffer_minutes": buffer})),
                Some(&owner),
            )
            .await
        }
    };
    for bad in [json!(-1), json!(121), json!(100000)] {
        assert_eq!(
            create(bad.clone()).await.0,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    assert_eq!(create(json!(0)).await.0, StatusCode::CREATED);
    assert_eq!(create(json!(120)).await.0, StatusCode::CREATED);
    // 沒給就是 0
    let (_, plain) = call(
        &s.app,
        Method::POST,
        "/t/shop-a/services",
        Some(json!({"name": "預設", "duration_minutes": 30})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(plain["buffer_minutes"], 0);
    // 更新同樣驗證範圍
    let patch = |buffer: Value| {
        let (app, owner, id) = (s.app.clone(), s.owner.clone(), s.cut.clone());
        async move {
            call(
                &app,
                Method::PATCH,
                &format!("/t/shop-a/services/{id}"),
                Some(json!({"buffer_minutes": buffer})),
                Some(&owner),
            )
            .await
            .0
        }
    };
    assert_eq!(patch(json!(-5)).await, StatusCode::BAD_REQUEST);
    assert_eq!(patch(json!(121)).await, StatusCode::BAD_REQUEST);
    assert_eq!(patch(json!(45)).await, StatusCode::OK);

    // 列表帶出來;員工不能改
    let (_, list) = call(
        &s.app,
        Method::GET,
        "/t/shop-a/services?limit=100",
        None,
        Some(&s.owner),
    )
    .await;
    let cut = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == s.cut.as_str())
        .unwrap();
    assert_eq!(cut["buffer_minutes"], 45);
    let staff = signup(&s.app, "staff@example.com").await;
    add_member(&pool, "shop-a", "staff@example.com", "staff").await;
    let (status, _) = call(
        &s.app,
        Method::PATCH,
        &format!("/t/shop-a/services/{}", s.cut),
        Some(json!({"buffer_minutes": 0})),
        Some(&staff),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 顧客看到的公開服務列表不含整理時間(那是店家內部的安排)
    let (_, public) = call(
        &s.app,
        Method::GET,
        "/public/shops/shop-a/services",
        None,
        None,
    )
    .await;
    assert!(public.to_string().contains("duration_minutes"));
    assert!(!public.to_string().contains("buffer"), "{public}");
}

/// migration 對「這個功能之前就存在的預約」:回填 `blocked_until = ends_at`(沒有整理時間),
/// 而且套用之後它們依然互不衝突、行為與以前一樣。
///
/// sqlx 的測試資料庫是套用完所有 migration 才交給測試的,沒辦法「先放資料再套用」;
/// 所以這裡把 0025 的回填語句抽出來,在一個 `blocked_until` 還是空的狀態下重跑一次:
/// 暫時拿掉 NOT NULL、塞入「舊式」資料(blocked_until 為 NULL)、執行回填、再檢查。
#[sqlx::test]
async fn the_backfill_gives_existing_bookings_no_buffer(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    let customer: Uuid = sqlx::query_scalar("INSERT INTO customers (tenant_id, name, email) VALUES ($1, 'c', 'c@example.com') RETURNING id")
        .bind(s.tenant_id).fetch_one(&pool).await.unwrap();

    // 還原成「舊結構」:blocked_until 可以是 NULL、trigger 先拿掉(它會自動補值,蓋掉我們要測的情境)
    // 排除約束也要拿掉:blocked_until 為 NULL 時範圍是「無限」,後面的預約都會被判定衝突
    sqlx::query("ALTER TABLE bookings DROP CONSTRAINT bookings_no_overlap")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TRIGGER bookings_blocked_until ON bookings")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE bookings ALTER COLUMN blocked_until DROP NOT NULL")
        .execute(&pool)
        .await
        .unwrap();
    for (h, m) in [(10, 0), (10, 30), (11, 0)] {
        sqlx::query(
            "INSERT INTO bookings (tenant_id, staff_user_id, service_id, customer_id, starts_at, ends_at,
                                   blocked_until, status, manage_token_hash)
             VALUES ($1, $2, $3, $4, $5, $6, NULL, 'confirmed', md5(random()::text) || md5(random()::text))",
        )
        .bind(s.tenant_id).bind(s.owner_id).bind(s.cut.parse::<Uuid>().unwrap()).bind(customer)
        .bind(at(d, h, m)).bind(at(d, h, m) + Duration::minutes(30))
        .execute(&pool).await.unwrap();
    }

    // 執行 0025 裡真正的回填語句(直接從 migration 檔取出,不是我手抄一份:抄的會跟著錯)
    let migration = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/migrations/0025_service_buffer.sql"
    ))
    .unwrap();
    let backfill: String = migration
        .lines()
        .find(|l| {
            l.trim_start()
                .starts_with("UPDATE bookings SET blocked_until")
        })
        .expect("migration 裡要有回填語句")
        .to_string();
    // 內容來自版本控制裡的 migration 檔,不是使用者輸入,所以明確標示為安全
    sqlx::query(sqlx::AssertSqlSafe(backfill))
        .execute(&pool)
        .await
        .unwrap();

    let rows: Vec<(DateTime<Utc>, DateTime<Utc>)> =
        sqlx::query_as("SELECT ends_at, blocked_until FROM bookings ORDER BY starts_at")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter().all(|(end, blocked)| end == blocked),
        "回填後 blocked_until = ends_at: {rows:?}"
    );
}
