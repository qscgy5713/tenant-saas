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
