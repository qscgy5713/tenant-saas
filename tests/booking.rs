mod common;

use axum::{
    Router,
    http::{Method, StatusCode},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Asia::Taipei;
use common::{
    add_member, app, app_with_rate_limit, call, create_service, create_tenant, signup, user_id,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tenant_saas::availability::local_to_utc;
use uuid::Uuid;

/// 店家本地(台北)的某天 hh:mm,換成 UTC
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
    service_id: String,
    owner_id: Uuid,
}

/// 開店(台北時區)、建立一個 60 分鐘的服務,並讓 owner 每天 09:00–18:00 提供該服務
async fn shop(pool: &PgPool) -> Shop {
    shop_on(app(pool.clone()), pool).await
}

async fn shop_on(app: Router, pool: &PgPool) -> Shop {
    let owner = signup(&app, "a@example.com").await;
    assert_eq!(
        create_tenant(&app, &owner, "shop-a").await.0,
        StatusCode::CREATED
    );
    let service = create_service(&app, &owner, "shop-a", "剪髮").await;
    let service_id = service["id"].as_str().unwrap().to_string();
    // create_service 建立的是 30 分鐘,改成 60 分鐘
    call(
        &app,
        Method::PATCH,
        &format!("/t/shop-a/services/{service_id}"),
        Some(json!({"duration_minutes": 60})),
        Some(&owner),
    )
    .await;
    let owner_id = user_id(pool, "a@example.com").await;
    open_every_day(&app, &owner, owner_id, &service_id).await;
    Shop {
        app,
        owner,
        service_id,
        owner_id,
    }
}

async fn open_every_day(app: &Router, owner_token: &str, staff_id: Uuid, service_id: &str) {
    let hours: Vec<Value> = (0..7)
        .map(|d| json!({"weekday": d, "start": "09:00", "end": "18:00"}))
        .collect();
    let (s, _) = call(
        app,
        Method::PUT,
        &format!("/t/shop-a/members/{staff_id}/working-hours"),
        Some(json!({"hours": hours})),
        Some(owner_token),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(
        app,
        Method::PUT,
        &format!("/t/shop-a/members/{staff_id}/services"),
        Some(json!({"service_ids": [service_id]})),
        Some(owner_token),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
}

/// 加入第二位員工 b,同樣每天營業並提供該服務
async fn add_second_staff(pool: &PgPool, s: &Shop) -> (String, Uuid) {
    let token = signup(&s.app, "b@example.com").await;
    add_member(pool, "shop-a", "b@example.com", "staff").await;
    let id = user_id(pool, "b@example.com").await;
    open_every_day(&s.app, &s.owner, id, &s.service_id).await;
    (token, id)
}

/// 以 owner 代客預約(直接確認)。顧客自助預約需要 Email 確認,見 tests/worker.rs。
async fn book(
    app: &Router,
    s: &Shop,
    staff: Option<Uuid>,
    start: DateTime<Utc>,
    email: &str,
) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/t/shop-a/bookings",
        Some(json!({
            "service_id": s.service_id,
            "staff_id": staff,
            "start": start.to_rfc3339(),
            "customer": {"name": "顧客", "email": email, "phone": "0912"},
        })),
        Some(&s.owner),
    )
    .await
}

async fn slots(app: &Router, s: &Shop, d: NaiveDate) -> Vec<Value> {
    let uri = format!(
        "/public/shops/shop-a/availability?service_id={}&from={d}",
        s.service_id
    );
    let (status, body) = call(app, Method::GET, &uri, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["slots"].as_array().unwrap().clone()
}

fn has_start(slots: &[Value], start: DateTime<Utc>) -> bool {
    slots.iter().any(|s| {
        s["start"]
            .as_str()
            .unwrap()
            .parse::<DateTime<Utc>>()
            .unwrap()
            == start
    })
}

#[sqlx::test]
async fn public_browsing_and_availability(pool: PgPool) {
    let s = shop(&pool).await;
    let app = &s.app;

    let (status, shop_info) = call(app, Method::GET, "/public/shops/shop-a", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(shop_info["timezone"], "Asia/Taipei");
    assert_eq!(
        call(app, Method::GET, "/public/shops/nope-nope", None, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let (_, services) = call(
        app,
        Method::GET,
        "/public/shops/shop-a/services",
        None,
        None,
    )
    .await;
    assert_eq!(services.as_array().unwrap().len(), 1);
    let (_, staff) = call(
        app,
        Method::GET,
        &format!("/public/shops/shop-a/services/{}/staff", s.service_id),
        None,
        None,
    )
    .await;
    assert_eq!(staff.as_array().unwrap().len(), 1);
    assert!(staff[0].get("email").is_none(), "不可洩漏員工 Email");

    // 09:00–18:00、60 分鐘、15 分鐘一格 → 09:00 … 17:00 共 33 個
    let d = day(3);
    let list = slots(app, &s, d).await;
    assert_eq!(list.len(), 33);
    assert!(
        has_start(&list, at(d, 9, 0))
            && has_start(&list, at(d, 17, 0))
            && !has_start(&list, at(d, 17, 15))
    );

    // 參數檢查
    let svc = &s.service_id;
    let bad = [
        format!(
            "/public/shops/shop-a/availability?service_id={svc}&from={}&to={}",
            day(5),
            day(3)
        ),
        format!(
            "/public/shops/shop-a/availability?service_id={svc}&from={}&to={}",
            day(1),
            day(20)
        ),
        format!(
            "/public/shops/shop-a/availability?service_id={}&from={}",
            Uuid::new_v4(),
            day(1)
        ),
    ];
    for uri in bad {
        assert_eq!(
            call(app, Method::GET, &uri, None, None).await.0,
            StatusCode::BAD_REQUEST,
            "{uri}"
        );
    }

    // 停用的服務不能查也不能訂
    call(
        app,
        Method::PATCH,
        &format!("/t/shop-a/services/{svc}"),
        Some(json!({"active": false})),
        Some(&s.owner),
    )
    .await;
    let (_, services) = call(
        app,
        Method::GET,
        "/public/shops/shop-a/services",
        None,
        None,
    )
    .await;
    assert_eq!(services.as_array().unwrap().len(), 0);
    assert_eq!(
        book(app, &s, None, at(d, 10, 0), "c@example.com").await.0,
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test]
async fn book_then_cancel_frees_the_slot(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    let start = at(d, 10, 0);

    let (status, created) = book(&s.app, &s, None, start, "c@example.com").await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let token = created["manage_token"].as_str().unwrap().to_string();

    // 資料庫只存雜湊
    let stored: String = sqlx::query_scalar("SELECT manage_token_hash FROM bookings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(stored, token);

    // 這個時段與所有重疊的時段都不再可預約;相鄰的仍可
    let list = slots(&s.app, &s, d).await;
    assert!(
        !has_start(&list, start)
            && !has_start(&list, at(d, 9, 15))
            && !has_start(&list, at(d, 10, 45))
    );
    assert!(has_start(&list, at(d, 9, 0)) && has_start(&list, at(d, 11, 0)));
    assert_eq!(
        book(&s.app, &s, None, at(d, 10, 30), "d@example.com")
            .await
            .0,
        StatusCode::CONFLICT
    );

    // 顧客用 token 查看
    let (status, view) = call(
        &s.app,
        Method::GET,
        &format!("/public/bookings/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["status"], "confirmed");
    assert_eq!(view["service_name"], "剪髮");
    // 前端改期需要:查可預約時段用的店家代稱、服務與員工 id
    assert_eq!(view["shop_slug"], "shop-a");
    assert_eq!(view["service_id"], s.service_id.as_str());
    assert_eq!(view["staff_id"], s.owner_id.to_string());
    assert!(view.get("customer_email").is_none());

    // 取消後時段釋出,可再被預約;重複取消 409
    let (status, view) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/cancel"),
        None,
        None,
    )
    .await;
    assert_eq!(
        (status, view["status"].as_str()),
        (StatusCode::OK, Some("cancelled"))
    );
    assert_eq!(
        call(
            &s.app,
            Method::POST,
            &format!("/public/bookings/{token}/cancel"),
            None,
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(has_start(&slots(&s.app, &s, d).await, start));
    assert_eq!(
        book(&s.app, &s, None, start, "d@example.com").await.0,
        StatusCode::CREATED
    );

    // 無效 token
    for bad in ["x".repeat(64), "short".to_string()] {
        assert_eq!(
            call(
                &s.app,
                Method::GET,
                &format!("/public/bookings/{bad}"),
                None,
                None
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
}

#[sqlx::test]
async fn reschedule(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    let (_, created) = book(&s.app, &s, None, at(d, 10, 0), "c@example.com").await;
    let token = created["manage_token"].as_str().unwrap();
    let uri = format!("/public/bookings/{token}/reschedule");
    let to = |t: DateTime<Utc>| Some(json!({"start": t.to_rfc3339()}));

    // 小幅移動(和自己原本的時段重疊)必須允許
    let (status, view) = call(&s.app, Method::POST, &uri, to(at(d, 10, 15)), None).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(
        view["starts_at"]
            .as_str()
            .unwrap()
            .parse::<DateTime<Utc>>()
            .unwrap(),
        at(d, 10, 15)
    );
    assert_eq!(
        view["ends_at"]
            .as_str()
            .unwrap()
            .parse::<DateTime<Utc>>()
            .unwrap(),
        at(d, 11, 15)
    );

    // 不在營業時間、過去、被別人占走
    assert_eq!(
        call(&s.app, Method::POST, &uri, to(at(d, 20, 0)), None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &s.app,
            Method::POST,
            &uri,
            to(Utc::now() - Duration::hours(1)),
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    book(&s.app, &s, None, at(d, 14, 0), "d@example.com").await;
    assert_eq!(
        call(&s.app, Method::POST, &uri, to(at(d, 14, 30)), None)
            .await
            .0,
        StatusCode::CONFLICT
    );

    // 取消後不能再改期
    call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/cancel"),
        None,
        None,
    )
    .await;
    assert_eq!(
        call(&s.app, Method::POST, &uri, to(at(d, 12, 0)), None)
            .await
            .0,
        StatusCode::CONFLICT
    );
}

#[sqlx::test]
async fn booking_validation(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    let ok = at(d, 10, 0);

    assert_eq!(
        book(
            &s.app,
            &s,
            None,
            Utc::now() - Duration::hours(1),
            "c@example.com"
        )
        .await
        .0,
        StatusCode::BAD_REQUEST,
        "過去"
    );
    assert_eq!(
        book(
            &s.app,
            &s,
            None,
            Utc::now() + Duration::days(120),
            "c@example.com"
        )
        .await
        .0,
        StatusCode::BAD_REQUEST,
        "太遠"
    );
    assert_eq!(
        book(&s.app, &s, None, at(d, 10, 7), "c@example.com")
            .await
            .0,
        StatusCode::CONFLICT,
        "不在 15 分鐘格線上"
    );
    assert_eq!(
        book(&s.app, &s, None, at(d, 7, 0), "c@example.com").await.0,
        StatusCode::CONFLICT,
        "營業前"
    );
    assert_eq!(
        book(&s.app, &s, None, at(d, 17, 30), "c@example.com")
            .await
            .0,
        StatusCode::CONFLICT,
        "會超過下班時間"
    );
    assert_eq!(
        book(&s.app, &s, None, ok, "not-an-email").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        book(&s.app, &s, Some(Uuid::new_v4()), ok, "c@example.com")
            .await
            .0,
        StatusCode::CONFLICT,
        "不提供該服務的員工"
    );

    let (status, _) = call(&s.app, Method::POST, "/public/shops/shop-a/bookings", Some(json!({
        "service_id": s.service_id, "start": ok.to_rfc3339(), "customer": {"name": "  ", "email": "c@example.com"}
    })), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "姓名空白");

    // 休假時段不可預約
    let off = json!({"starts_at": at(d, 13, 0).to_rfc3339(), "ends_at": at(d, 15, 0).to_rfc3339()});
    call(
        &s.app,
        Method::POST,
        &format!("/t/shop-a/members/{}/time-off", s.owner_id),
        Some(off),
        Some(&s.owner),
    )
    .await;
    let list = slots(&s.app, &s, d).await;
    assert!(!has_start(&list, at(d, 13, 30)) && !has_start(&list, at(d, 12, 15)));
    assert!(has_start(&list, at(d, 12, 0)) && has_start(&list, at(d, 15, 0)));
    assert_eq!(
        book(&s.app, &s, None, at(d, 13, 30), "c@example.com")
            .await
            .0,
        StatusCode::CONFLICT
    );
}

#[sqlx::test]
async fn concurrent_requests_for_the_same_slot_only_one_wins(pool: PgPool) {
    let s = shop(&pool).await;
    let start = at(day(3), 10, 0);

    let mut handles = Vec::new();
    for i in 0..8 {
        let (app, svc, owner) = (s.app.clone(), s.service_id.clone(), s.owner.clone());
        handles.push(tokio::spawn(async move {
            call(
                &app,
                Method::POST,
                "/t/shop-a/bookings",
                Some(json!({
                    "service_id": svc, "staff_id": null, "start": start.to_rfc3339(),
                    "customer": {"name": "顧客", "email": format!("c{i}@example.com")},
                })),
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
    let conflicts = results
        .iter()
        .filter(|s| **s == StatusCode::CONFLICT)
        .count();
    assert_eq!((created, conflicts), (1, 7), "{results:?}");

    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM bookings WHERE status = 'confirmed'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test]
async fn any_staff_falls_back_to_the_next_free_staff(pool: PgPool) {
    let s = shop(&pool).await;
    add_second_staff(&pool, &s).await;
    let start = at(day(3), 10, 0);

    // 同時段、不指定員工:8 個併發請求,兩位員工 → 恰好 2 個成功
    let mut handles = Vec::new();
    for i in 0..8 {
        let (app, svc, owner) = (s.app.clone(), s.service_id.clone(), s.owner.clone());
        handles.push(tokio::spawn(async move {
            call(
                &app,
                Method::POST,
                "/t/shop-a/bookings",
                Some(json!({
                    "service_id": svc, "start": start.to_rfc3339(),
                    "customer": {"name": "顧客", "email": format!("c{i}@example.com")},
                })),
                Some(&owner),
            )
            .await
        }));
    }
    let mut staff_ids = Vec::new();
    let mut conflicts = 0;
    for h in handles {
        let (status, body) = h.await.unwrap();
        match status {
            StatusCode::CREATED => staff_ids.push(body["staff_id"].as_str().unwrap().to_string()),
            StatusCode::CONFLICT => conflicts += 1,
            other => panic!("非預期狀態 {other}: {body}"),
        }
    }
    staff_ids.sort();
    staff_ids.dedup();
    assert_eq!((staff_ids.len(), conflicts), (2, 6), "兩位員工各被分配一筆");
}

#[sqlx::test]
async fn customer_limits_and_data_protection(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);

    // 同一個 Email 最多 5 筆未來預約
    for h in 9..14 {
        assert_eq!(
            book(&s.app, &s, None, at(d, h, 0), "same@example.com")
                .await
                .0,
            StatusCode::CREATED
        );
    }
    assert_eq!(
        book(&s.app, &s, None, at(d, 15, 0), "same@example.com")
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        book(&s.app, &s, None, at(d, 15, 0), "other@example.com")
            .await
            .0,
        StatusCode::CREATED,
        "別人不受影響"
    );

    // 冒用別人的 Email 預約,不能改掉那位顧客既有的姓名 / 電話
    call(
        &s.app,
        Method::POST,
        "/public/shops/shop-a/bookings",
        Some(json!({
            "service_id": s.service_id, "start": at(d, 16, 0).to_rfc3339(),
            "customer": {"name": "冒用者", "email": "other@example.com", "phone": "9999"},
        })),
        None,
    )
    .await;
    let (name, phone): (String, Option<String>) =
        sqlx::query_as("SELECT name, phone FROM customers WHERE email = 'other@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((name.as_str(), phone.as_deref()), ("顧客", Some("0912")));
}

#[sqlx::test]
async fn staff_side_bookings(pool: PgPool) {
    let s = shop(&pool).await;
    let (staff_token, staff_id) = add_second_staff(&pool, &s).await;
    let d = day(3);
    book(&s.app, &s, Some(s.owner_id), at(d, 10, 0), "c1@example.com").await;
    book(&s.app, &s, Some(staff_id), at(d, 10, 0), "c2@example.com").await;

    // owner 看全部;staff 只看自己的
    let (_, all) = call(
        &s.app,
        Method::GET,
        "/t/shop-a/bookings",
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(all["items"].as_array().unwrap().len(), 2);
    let (_, mine) = call(
        &s.app,
        Method::GET,
        "/t/shop-a/bookings",
        None,
        Some(&staff_token),
    )
    .await;
    assert_eq!(mine["items"].as_array().unwrap().len(), 1);
    assert_eq!(mine["items"][0]["customer_email"], "c2@example.com");
    let other = format!("/t/shop-a/bookings?staff_id={}", s.owner_id);
    assert_eq!(
        call(&s.app, Method::GET, &other, None, Some(&staff_token))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (_, filtered) = call(
        &s.app,
        Method::GET,
        &format!("/t/shop-a/bookings?staff_id={staff_id}&status=confirmed"),
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(filtered["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        call(&s.app, Method::GET, "/t/shop-a/bookings", None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );

    // 代客預約:staff 不能替別人建立
    let body = |staff: Uuid, h: u32| {
        Some(json!({
            "service_id": s.service_id, "staff_id": staff, "start": at(d, h, 0).to_rfc3339(),
            "customer": {"name": "電話客", "email": "phone@example.com"},
        }))
    };
    assert_eq!(
        call(
            &s.app,
            Method::POST,
            "/t/shop-a/bookings",
            body(s.owner_id, 13),
            Some(&staff_token)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &s.app,
            Method::POST,
            "/t/shop-a/bookings",
            body(staff_id, 13),
            Some(&staff_token)
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(
            &s.app,
            Method::POST,
            "/t/shop-a/bookings",
            body(s.owner_id, 13),
            Some(&s.owner)
        )
        .await
        .0,
        StatusCode::CREATED
    );

    // 狀態:不能改回 confirmed、未開始不能標完成、staff 不能動別人的
    let (_, owners) = call(
        &s.app,
        Method::GET,
        &format!("/t/shop-a/bookings?staff_id={}", s.owner_id),
        None,
        Some(&s.owner),
    )
    .await;
    let id = owners["items"][0]["id"].as_str().unwrap().to_string();
    let uri = format!("/t/shop-a/bookings/{id}");
    let patch = |token: &str, b: Value| {
        let (a, t, u) = (s.app.clone(), token.to_string(), uri.clone());
        async move { call(&a, Method::PATCH, &u, Some(b), Some(&t)).await }
    };
    assert_eq!(
        patch(&staff_token, json!({"notes": "x"})).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        patch(&s.owner, json!({"status": "confirmed"})).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        patch(&s.owner, json!({"status": "completed"})).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        patch(&s.owner, json!({"notes": "n".repeat(501)})).await.0,
        StatusCode::BAD_REQUEST
    );
    let (status, updated) = patch(&s.owner, json!({"notes": "VIP"})).await;
    assert_eq!(
        (status, updated["notes"].as_str()),
        (StatusCode::OK, Some("VIP"))
    );

    // 開始時間過了之後才能標完成;終態不能再改
    sqlx::query("UPDATE bookings SET starts_at = now() - interval '2 hours', ends_at = now() - interval '1 hour' WHERE id = $1::uuid")
        .bind(&id).execute(&pool).await.unwrap();
    assert_eq!(
        patch(&s.owner, json!({"status": "completed"})).await.0,
        StatusCode::OK
    );
    assert_eq!(
        patch(&s.owner, json!({"status": "cancelled"})).await.0,
        StatusCode::CONFLICT
    );

    // 取消會釋出時段
    let (_, mine) = call(
        &s.app,
        Method::GET,
        "/t/shop-a/bookings?status=confirmed",
        None,
        Some(&staff_token),
    )
    .await;
    let sid = mine["items"][0]["id"].as_str().unwrap();
    assert_eq!(
        call(
            &s.app,
            Method::PATCH,
            &format!("/t/shop-a/bookings/{sid}"),
            Some(json!({"status": "cancelled"})),
            Some(&staff_token)
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn bookings_are_isolated_between_shops(pool: PgPool) {
    let s = shop(&pool).await;
    // 同一個人也擁有 shop-b
    create_tenant(&s.app, &s.owner, "shop-b").await;
    let d = day(3);
    let (_, created) = book(&s.app, &s, None, at(d, 10, 0), "c@example.com").await;
    let token = created["manage_token"].as_str().unwrap();

    let (_, in_b) = call(
        &s.app,
        Method::GET,
        "/t/shop-b/bookings",
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(
        in_b["items"].as_array().unwrap().len(),
        0,
        "同一個人,B 店的上下文看不到 A 店的預約"
    );
    let id = created["id"].as_str().unwrap();
    assert_eq!(
        call(
            &s.app,
            Method::PATCH,
            &format!("/t/shop-b/bookings/{id}"),
            Some(json!({"notes": "x"})),
            Some(&s.owner)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    // 拿 A 店的服務 id 到 B 店預約:失敗
    let (status, _) = call(&s.app, Method::POST, "/public/shops/shop-b/bookings", Some(json!({
        "service_id": s.service_id, "start": at(d, 11, 0).to_rfc3339(), "customer": {"name": "x", "email": "x@example.com"},
    })), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let uri = format!(
        "/public/shops/shop-b/availability?service_id={}&from={d}",
        s.service_id
    );
    assert_eq!(
        call(&s.app, Method::GET, &uri, None, None).await.0,
        StatusCode::BAD_REQUEST
    );

    // token 只能管理自己的那一筆
    let (_, view) = call(
        &s.app,
        Method::GET,
        &format!("/public/bookings/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(view["shop_name"], "店 shop-a");
}

#[sqlx::test]
async fn exclusion_constraint_is_the_last_line_of_defense(pool: PgPool) {
    let s = shop(&pool).await;
    let (tenant, customer): (Uuid, Uuid) = {
        let tenant: Uuid = sqlx::query_scalar("SELECT id FROM tenants WHERE slug = 'shop-a'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let customer: Uuid = sqlx::query_scalar("INSERT INTO customers (tenant_id, name, email) VALUES ($1, 'x', 'x@example.com') RETURNING id").bind(tenant).fetch_one(&pool).await.unwrap();
        (tenant, customer)
    };
    let service: Uuid = s.service_id.parse().unwrap();
    let d = day(5);
    let insert = |from: DateTime<Utc>, to: DateTime<Utc>, status: &'static str, n: u32| {
        let pool = pool.clone();
        let hash = format!("hash-{n}");
        async move {
            sqlx::query("INSERT INTO bookings (tenant_id, staff_user_id, service_id, customer_id, starts_at, ends_at, status, manage_token_hash) VALUES ($1, $2, $3, $4, $5, $6, $7::booking_status, $8)")
                .bind(tenant).bind(s.owner_id).bind(service).bind(customer).bind(from).bind(to).bind(status).bind(hash)
                .execute(&pool).await
        }
    };

    insert(at(d, 10, 0), at(d, 11, 0), "confirmed", 1)
        .await
        .unwrap();
    let err = insert(at(d, 10, 30), at(d, 11, 30), "confirmed", 2)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23P01")
    );
    insert(at(d, 11, 0), at(d, 12, 0), "confirmed", 3)
        .await
        .unwrap(); // 相鄰不算重疊
    insert(at(d, 10, 0), at(d, 11, 0), "cancelled", 4)
        .await
        .unwrap(); // 已取消的不占時段
}

#[sqlx::test]
async fn member_with_bookings_cannot_be_removed(pool: PgPool) {
    let s = shop(&pool).await;
    let (_, staff_id) = add_second_staff(&pool, &s).await;
    book(
        &s.app,
        &s,
        Some(staff_id),
        at(day(3), 10, 0),
        "c@example.com",
    )
    .await;
    let (status, _) = call(
        &s.app,
        Method::DELETE,
        &format!("/t/shop-a/members/{staff_id}"),
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // 有預約紀錄的服務也不能刪除
    let (status, _) = call(
        &s.app,
        Method::DELETE,
        &format!("/t/shop-a/services/{}", s.service_id),
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[sqlx::test]
async fn public_endpoints_are_rate_limited(pool: PgPool) {
    let s = shop_on(app_with_rate_limit(pool.clone(), 3), &pool).await;
    for _ in 0..3 {
        assert_eq!(
            call(&s.app, Method::GET, "/public/shops/shop-a", None, None)
                .await
                .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        call(&s.app, Method::GET, "/public/shops/shop-a", None, None)
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        call(
            &s.app,
            Method::GET,
            "/public/shops/shop-a/services",
            None,
            None
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    // 其他端點不受影響
    assert_eq!(
        call(&s.app, Method::GET, "/health", None, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&s.app, Method::GET, "/t/shop-a/me", None, Some(&s.owner))
            .await
            .0,
        StatusCode::OK
    );
}

// ---------- 改期:可預約時段排除自己 + 員工替顧客改期 ----------

#[sqlx::test]
async fn reschedule_availability_does_not_treat_the_booking_itself_as_busy(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    let (_, created) = book(&s.app, &s, None, at(d, 10, 0), "c@example.com").await;
    let token = created["manage_token"].as_str().unwrap();
    let id = created["id"].as_str().unwrap();

    // 一般的公開查詢:自己的 10:00 與重疊的 09:15–10:45 都是忙碌
    let general = slots(&s.app, &s, d).await;
    assert!(!has_start(&general, at(d, 10, 0)) && !has_start(&general, at(d, 10, 15)));

    // 改期專用:自己的時段是空的,連小幅調整(10:15)都選得到
    let uri = format!("/public/bookings/{token}/availability?from={d}");
    let (status, body) = call(&s.app, Method::GET, &uri, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let own = body["slots"].as_array().unwrap().clone();
    assert!(
        has_start(&own, at(d, 10, 0))
            && has_start(&own, at(d, 10, 15))
            && has_start(&own, at(d, 9, 15))
    );

    // 別人的預約仍然是忙碌
    book(&s.app, &s, None, at(d, 14, 0), "other@example.com").await;
    let (_, body) = call(&s.app, Method::GET, &uri, None, None).await;
    assert!(!has_start(body["slots"].as_array().unwrap(), at(d, 14, 0)));

    // 員工端同樣
    let staff_uri = format!("/t/shop-a/bookings/{id}/availability?from={d}");
    let (status, body) = call(&s.app, Method::GET, &staff_uri, None, Some(&s.owner)).await;
    assert_eq!(status, StatusCode::OK);
    let slots = body["slots"].as_array().unwrap();
    assert!(has_start(slots, at(d, 10, 15)) && !has_start(slots, at(d, 14, 0)));

    // 參數與狀態檢查
    assert_eq!(
        call(
            &s.app,
            Method::GET,
            &format!("{uri}&to={}", day(30)),
            None,
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &s.app,
            Method::GET,
            &format!("/public/bookings/{}/availability?from={d}", "x".repeat(64)),
            None,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/cancel"),
        None,
        None,
    )
    .await;
    assert_eq!(
        call(&s.app, Method::GET, &uri, None, None).await.0,
        StatusCode::CONFLICT,
        "已取消的不能改期"
    );
}

#[sqlx::test]
async fn staff_can_reschedule_and_the_customer_is_told(pool: PgPool) {
    let s = shop(&pool).await;
    let d = day(3);
    let (_, created) = book(&s.app, &s, None, at(d, 10, 0), "c@example.com").await;
    let id = created["id"].as_str().unwrap();
    let uri = format!("/t/shop-a/bookings/{id}/reschedule");

    // 小幅移動(和自己原本的時段重疊)必須允許
    let (status, body) = call(
        &s.app,
        Method::POST,
        &uri,
        Some(json!({"start": at(d, 10, 15).to_rfc3339()})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["starts_at"]
            .as_str()
            .unwrap()
            .parse::<DateTime<Utc>>()
            .unwrap(),
        at(d, 10, 15)
    );
    assert_eq!(
        body["ends_at"]
            .as_str()
            .unwrap()
            .parse::<DateTime<Utc>>()
            .unwrap(),
        at(d, 11, 15)
    );

    // 稽核與通知信
    let subjects: Vec<String> = sqlx::query_scalar(
        "SELECT subject FROM email_outbox WHERE dedupe_key LIKE 'rescheduled:%'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(subjects.len(), 1);
    assert!(subjects[0].contains("已變更"));
    let detail: serde_json::Value =
        sqlx::query_scalar("SELECT detail FROM audit_logs WHERE action = 'booking.rescheduled'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(detail["source"], "staff");

    // 規則與顧客改期相同:被別人占走 / 不在營業時間 / 過去
    book(&s.app, &s, None, at(d, 14, 0), "other@example.com").await;
    let try_at = |t: DateTime<Utc>| {
        let (app, owner, uri) = (s.app.clone(), s.owner.clone(), uri.clone());
        async move {
            call(
                &app,
                Method::POST,
                &uri,
                Some(json!({"start": t.to_rfc3339()})),
                Some(&owner),
            )
            .await
            .0
        }
    };
    assert_eq!(try_at(at(d, 14, 30)).await, StatusCode::CONFLICT);
    assert_eq!(try_at(at(d, 20, 0)).await, StatusCode::CONFLICT);
    assert_eq!(
        try_at(Utc::now() - Duration::hours(1)).await,
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test]
async fn reschedule_permissions_and_states(pool: PgPool) {
    let s = shop(&pool).await;
    let (staff_token, staff_id) = add_second_staff(&pool, &s).await;
    let d = day(3);
    let (_, owners) = book(&s.app, &s, Some(s.owner_id), at(d, 10, 0), "a@example.com").await;
    let (_, theirs) = book(&s.app, &s, Some(staff_id), at(d, 10, 0), "b@example.com").await;
    let body = |t: DateTime<Utc>| Some(json!({"start": t.to_rfc3339()}));
    let reschedule = |token: &str, id: &Value, t: DateTime<Utc>| {
        let (app, token, id) = (
            s.app.clone(),
            token.to_string(),
            id.as_str().unwrap().to_string(),
        );
        let b = body(t);
        async move {
            call(
                &app,
                Method::POST,
                &format!("/t/shop-a/bookings/{id}/reschedule"),
                b,
                Some(&token),
            )
            .await
            .0
        }
    };

    // 員工只能動自己的;管理者什麼都能動
    assert_eq!(
        reschedule(&staff_token, &owners["id"], at(d, 12, 0)).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        reschedule(&staff_token, &theirs["id"], at(d, 12, 0)).await,
        StatusCode::OK
    );
    assert_eq!(
        reschedule(&s.owner, &theirs["id"], at(d, 13, 0)).await,
        StatusCode::OK
    );
    let uri = format!(
        "/t/shop-a/bookings/{}/availability?from={d}",
        owners["id"].as_str().unwrap()
    );
    assert_eq!(
        call(&s.app, Method::GET, &uri, None, Some(&staff_token))
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    // 找不到 / 未登入
    assert_eq!(
        reschedule(&s.owner, &json!(Uuid::new_v4().to_string()), at(d, 12, 0)).await,
        StatusCode::NOT_FOUND
    );
    let (status, _) = call(
        &s.app,
        Method::POST,
        &format!(
            "/t/shop-a/bookings/{}/reschedule",
            owners["id"].as_str().unwrap()
        ),
        body(at(d, 12, 0)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 已取消的不能改期
    call(
        &s.app,
        Method::PATCH,
        &format!("/t/shop-a/bookings/{}", owners["id"].as_str().unwrap()),
        Some(json!({"status": "cancelled"})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(
        reschedule(&s.owner, &owners["id"], at(d, 12, 0)).await,
        StatusCode::CONFLICT
    );
}

#[sqlx::test]
async fn staff_reschedule_into_a_full_month_reports_the_limit(pool: PgPool) {
    let s = shop(&pool).await;
    sqlx::query("UPDATE plans SET max_bookings_per_month = 1 WHERE id = 'free'")
        .execute(&pool)
        .await
        .unwrap();
    let first = at(day(5), 10, 0);
    book(&s.app, &s, None, first, "a@example.com").await; // 占滿這個月唯一的名額
    let other_month = at(day(5) + Duration::days(40), 10, 0);
    let (_, created) = book(&s.app, &s, None, other_month, "b@example.com").await;
    let (status, body) = call(
        &s.app,
        Method::POST,
        &format!(
            "/t/shop-a/bookings/{}/reschedule",
            created["id"].as_str().unwrap()
        ),
        Some(json!({"start": (first + Duration::hours(2)).to_rfc3339()})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::PAYMENT_REQUIRED,
        "員工看得到方案資訊: {body}"
    );
}

// ---------- 取消預約的通知 ----------

/// 取消通知信:(dedupe_key, 收件者, 標題, 內容),依 dedupe_key 排序
async fn cancel_mails(pool: &PgPool) -> Vec<(String, String, String, String)> {
    sqlx::query_as(
        "SELECT dedupe_key, to_email, subject, body FROM email_outbox
         WHERE dedupe_key LIKE 'cancelled:%' ORDER BY dedupe_key",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn cancel_as(s: &Shop, token: &str, id: &str) -> (StatusCode, Value) {
    call(
        &s.app,
        Method::PATCH,
        &format!("/t/shop-a/bookings/{id}"),
        Some(json!({"status": "cancelled"})),
        Some(token),
    )
    .await
}

#[sqlx::test]
async fn shop_cancelling_a_confirmed_booking_tells_the_customer(pool: PgPool) {
    let s = shop(&pool).await;
    let (_, created) = book(&s.app, &s, None, at(day(3), 10, 0), "c@example.com").await;
    let id = created["id"].as_str().unwrap();

    assert_eq!(cancel_as(&s, &s.owner, id).await.0, StatusCode::OK);

    let mails = cancel_mails(&pool).await;
    // 負責的員工就是取消的人自己 → 只通知顧客
    assert_eq!(mails.len(), 1, "{mails:?}");
    let (key, to, subject, body) = &mails[0];
    assert_eq!(key, &format!("cancelled:{id}:customer"));
    assert_eq!(to, "c@example.com");
    assert!(
        subject.contains("預約已取消") && subject.contains("店 shop-a"),
        "{subject}"
    );
    assert!(body.contains("剪髮") && body.contains("10:00"), "{body}");
    // 給顧客一條路:回店家的預約頁重新預約
    assert!(body.contains("http://app.test/s/shop-a"), "{body}");

    // 已經是終態,再取消是 409,不會再寄一封
    assert_eq!(cancel_as(&s, &s.owner, id).await.0, StatusCode::CONFLICT);
    assert_eq!(cancel_mails(&pool).await.len(), 1);
}

#[sqlx::test]
async fn manager_cancelling_someone_elses_booking_also_tells_that_staff(pool: PgPool) {
    let s = shop(&pool).await;
    let (b_token, b_id) = add_second_staff(&pool, &s).await;
    let (_, created) = book(&s.app, &s, Some(b_id), at(day(3), 10, 0), "c@example.com").await;
    let id = created["id"].as_str().unwrap();

    assert_eq!(cancel_as(&s, &s.owner, id).await.0, StatusCode::OK);
    let mails = cancel_mails(&pool).await;
    let recipients: Vec<_> = mails.iter().map(|m| (m.0.as_str(), m.1.as_str())).collect();
    assert_eq!(
        recipients,
        [
            (format!("cancelled:{id}:customer").as_str(), "c@example.com"),
            (format!("cancelled:{id}:staff").as_str(), "b@example.com"),
        ]
    );
    assert!(mails[1].3.contains("店家管理者取消"), "{}", mails[1].3);
    assert!(
        mails[1].3.contains("顧客"),
        "要讓員工知道是哪位顧客: {}",
        mails[1].3
    );

    // 員工取消自己負責的預約 → 只通知顧客
    let (_, again) = book(&s.app, &s, Some(b_id), at(day(4), 10, 0), "d@example.com").await;
    let id2 = again["id"].as_str().unwrap();
    assert_eq!(cancel_as(&s, &b_token, id2).await.0, StatusCode::OK);
    let keys: Vec<_> = cancel_mails(&pool).await.into_iter().map(|m| m.0).collect();
    assert!(keys.contains(&format!("cancelled:{id2}:customer")));
    assert!(
        !keys.contains(&format!("cancelled:{id2}:staff")),
        "{keys:?}"
    );
}

#[sqlx::test]
async fn customer_cancelling_tells_the_staff_not_themselves(pool: PgPool) {
    let s = shop(&pool).await;
    let (_, b_id) = add_second_staff(&pool, &s).await;
    let (_, created) = book(&s.app, &s, Some(b_id), at(day(3), 10, 0), "c@example.com").await;
    let (id, token) = (
        created["id"].as_str().unwrap(),
        created["manage_token"].as_str().unwrap(),
    );

    let (status, _) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/cancel"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let mails = cancel_mails(&pool).await;
    assert_eq!(mails.len(), 1, "顧客自己按的取消,不必再寄給他: {mails:?}");
    let (key, to, _, body) = &mails[0];
    assert_eq!(key, &format!("cancelled:{id}:staff"));
    assert_eq!(to, "b@example.com");
    assert!(body.contains("顧客自己取消"), "{body}");
    assert!(body.contains("剪髮") && body.contains("10:00"), "{body}");

    // 重複取消(重送、連點)→ 409,不會重複通知
    let (status, _) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/cancel"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(cancel_mails(&pool).await.len(), 1);
}

#[sqlx::test]
async fn unverified_and_past_bookings_send_no_cancel_mail(pool: PgPool) {
    let s = shop(&pool).await;

    // 待確認(Email 還沒驗證):店家取消 / 顧客取消都不寄,員工也沒看過它
    let (_, pending) = book(&s.app, &s, None, at(day(3), 10, 0), "p@example.com").await;
    let pid = pending["id"].as_str().unwrap();
    sqlx::query("UPDATE bookings SET status = 'pending' WHERE id = $1::uuid")
        .bind(pid)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(cancel_as(&s, &s.owner, pid).await.0, StatusCode::OK);

    let (_, pending2) = book(&s.app, &s, None, at(day(4), 10, 0), "q@example.com").await;
    let (qid, qtoken) = (
        pending2["id"].as_str().unwrap(),
        pending2["manage_token"].as_str().unwrap(),
    );
    sqlx::query("UPDATE bookings SET status = 'pending' WHERE id = $1::uuid")
        .bind(qid)
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{qtoken}/cancel"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 已經過去的預約被取消:通知「取消」只會造成困惑
    let (_, past) = book(&s.app, &s, None, at(day(5), 10, 0), "r@example.com").await;
    let rid = past["id"].as_str().unwrap();
    sqlx::query("UPDATE bookings SET starts_at = now() - interval '2 hours', ends_at = now() - interval '1 hour' WHERE id = $1::uuid")
        .bind(rid)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(cancel_as(&s, &s.owner, rid).await.0, StatusCode::OK);

    // 標記完成 / 未到不是取消
    let (_, done) = book(&s.app, &s, None, at(day(6), 10, 0), "s@example.com").await;
    let did = done["id"].as_str().unwrap();
    sqlx::query("UPDATE bookings SET starts_at = now() - interval '2 hours', ends_at = now() - interval '1 hour' WHERE id = $1::uuid")
        .bind(did)
        .execute(&pool)
        .await
        .unwrap();
    let (status, _) = call(
        &s.app,
        Method::PATCH,
        &format!("/t/shop-a/bookings/{did}"),
        Some(json!({"status": "completed"})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(cancel_mails(&pool).await.is_empty());
}
