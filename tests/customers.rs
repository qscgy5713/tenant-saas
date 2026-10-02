mod common;

use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use common::{add_member, app, call, create_service, create_tenant, signup, user_id};
use serde_json::{Value, json};
use sqlx::PgPool;

struct Shop {
    app: axum::Router,
    owner: String,
    owner_id: uuid::Uuid,
    service: String,
}

async fn shop(pool: &PgPool, slug: &str, email: &str) -> Shop {
    let app = app(pool.clone());
    let owner = signup(&app, email).await;
    create_tenant(&app, &owner, slug).await;
    let svc = create_service(&app, &owner, slug, "剪髮").await;
    let owner_id = user_id(pool, email).await;
    call(
        &app,
        Method::PUT,
        &format!("/t/{slug}/members/{owner_id}/services"),
        Some(json!({"service_ids": [svc["id"]]})),
        Some(&owner),
    )
    .await;
    let hours: Vec<Value> = (0..7)
        .map(|d| json!({"weekday": d, "start": "00:00", "end": "23:59"}))
        .collect();
    call(
        &app,
        Method::PUT,
        &format!("/t/{slug}/members/{owner_id}/working-hours"),
        Some(json!({"hours": hours})),
        Some(&owner),
    )
    .await;
    Shop {
        app,
        owner,
        owner_id,
        service: svc["id"].as_str().unwrap().to_string(),
    }
}

async fn book(s: &Shop, slug: &str, days: i64, hour: u32, name: &str, email: &str) -> String {
    let start = (Utc::now() + Duration::days(days))
        .format(&format!("%Y-%m-%dT{hour:02}:00:00Z"))
        .to_string();
    let (status, body) = call(&s.app, Method::POST, &format!("/t/{slug}/bookings"), Some(json!({"service_id": s.service, "start": start, "customer": {"name": name, "email": email, "phone": "0912-345-678"}})), Some(&s.owner)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_string()
}

#[sqlx::test]
async fn customers_are_listed_with_counts_search_and_history(pool: PgPool) {
    let s = shop(&pool, "shop-a", "o@example.com").await;
    let past = book(&s, "shop-a", 1, 9, "王小明", "ming@example.com").await;
    book(&s, "shop-a", 3, 9, "王小明", "ming@example.com").await;
    book(&s, "shop-a", 2, 9, "李小華", "hua@example.com").await;
    // 把第一筆改成過去並標記為完成,另一位的標記為未到
    sqlx::query("UPDATE bookings SET starts_at = now() - interval '5 days', ends_at = now() - interval '5 days' + interval '1 hour', status = 'completed' WHERE id = $1::uuid").bind(&past).execute(&pool).await.unwrap();
    sqlx::query("UPDATE bookings SET status = 'no_show', starts_at = now() - interval '2 days', ends_at = now() - interval '2 days' + interval '1 hour' WHERE customer_id = (SELECT id FROM customers WHERE name = '李小華')").execute(&pool).await.unwrap();

    let (status, body) = call(
        &s.app,
        Method::GET,
        "/t/shop-a/customers",
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let ming = items.iter().find(|c| c["name"] == "王小明").unwrap();
    assert_eq!(
        (
            ming["bookings"].as_i64(),
            ming["completed"].as_i64(),
            ming["no_shows"].as_i64()
        ),
        (Some(2), Some(1), Some(0))
    );
    assert!(ming["next_at"].is_string() && ming["last_at"].is_string());
    let hua = items.iter().find(|c| c["name"] == "李小華").unwrap();
    assert_eq!(hua["no_shows"], 1);
    assert!(hua["next_at"].is_null());
    // 最近有動靜的排前面:李小華 2 天前、王小明下次在 3 天後(以最新的預約時間排序)
    assert_eq!(items[0]["name"], "王小明");

    // 搜尋:名字 / Email / 電話;萬用字元要跳脫
    let search = |q: &str| {
        let (app, owner, q) = (s.app.clone(), s.owner.clone(), q.to_string());
        async move {
            let (_, b) = call(
                &app,
                Method::GET,
                &format!("/t/shop-a/customers?q={q}"),
                None,
                Some(&owner),
            )
            .await;
            b["items"].as_array().unwrap().len()
        }
    };
    assert_eq!(search("小華").await, 1);
    assert_eq!(search("MING@").await, 1, "不分大小寫");
    assert_eq!(search("345").await, 2);
    assert_eq!(search("%25").await, 0, "% 不能當萬用字元");
    assert_eq!(search("_").await, 0);
    assert_eq!(search("").await, 2, "空字串 = 不篩選");

    // 詳情:歷史依時間新到舊
    let id = ming["id"].as_str().unwrap();
    let (status, detail) = call(
        &s.app,
        Method::GET,
        &format!("/t/shop-a/customers/{id}"),
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let history = detail["history"].as_array().unwrap();
    assert_eq!(history.len(), 2);
    assert!(history[0]["starts_at"].as_str() > history[1]["starts_at"].as_str());
    assert_eq!(detail["email"], "ming@example.com");
}

#[sqlx::test]
async fn unconfirmed_requests_do_not_create_customer_entries(pool: PgPool) {
    let s = shop(&pool, "shop-a", "o@example.com").await;
    // 顧客自助預約(待確認):有人輸入了別人的 Email,不代表那個人真的來過
    let start = (Utc::now() + Duration::days(3))
        .format("%Y-%m-%dT10:00:00Z")
        .to_string();
    let (status, _) = call(&s.app, Method::POST, "/public/shops/shop-a/bookings", Some(json!({"service_id": s.service, "start": start, "customer": {"name": "冒名者", "email": "victim@example.com"}})), None).await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, body) = call(
        &s.app,
        Method::GET,
        "/t/shop-a/customers",
        None,
        Some(&s.owner),
    )
    .await;
    assert!(body["items"].as_array().unwrap().is_empty());

    // 取消過的不算「確認過」,但已確認後才取消的顧客歷史仍保留
    book(&s, "shop-a", 4, 9, "真顧客", "real@example.com").await;
    let (_, body) = call(
        &s.app,
        Method::GET,
        "/t/shop-a/customers",
        None,
        Some(&s.owner),
    )
    .await;
    let id = body["items"][0]["id"].as_str().unwrap().to_string();
    sqlx::query("UPDATE bookings SET status = 'cancelled' WHERE customer_id = $1::uuid")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    let (_, body) = call(
        &s.app,
        Method::GET,
        "/t/shop-a/customers",
        None,
        Some(&s.owner),
    )
    .await;
    assert!(
        body["items"].as_array().unwrap().is_empty(),
        "只剩取消的不列入"
    );
}

#[sqlx::test]
async fn only_managers_see_customers_and_never_across_tenants(pool: PgPool) {
    let a = shop(&pool, "shop-a", "a@example.com").await;
    book(&a, "shop-a", 2, 9, "A店顧客", "ca@example.com").await;
    let b = shop(&pool, "shop-b", "b@example.com").await;
    book(&b, "shop-b", 2, 9, "B店顧客", "cb@example.com").await;
    let _ = a.owner_id;

    let staff = signup(&a.app, "staff@example.com").await;
    add_member(&pool, "shop-a", "staff@example.com", "staff").await;
    let manager = signup(&a.app, "mgr@example.com").await;
    add_member(&pool, "shop-a", "mgr@example.com", "manager").await;

    assert_eq!(
        call(
            &a.app,
            Method::GET,
            "/t/shop-a/customers",
            None,
            Some(&staff)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, body) = call(
        &a.app,
        Method::GET,
        "/t/shop-a/customers",
        None,
        Some(&manager),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["A店顧客"]);

    // A 店的店主拿 B 店的顧客編號 → 404;不是成員直接開 B 店 → 404
    let b_customer: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM customers WHERE name = 'B店顧客'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        call(
            &a.app,
            Method::GET,
            &format!("/t/shop-a/customers/{b_customer}"),
            None,
            Some(&a.owner)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &a.app,
            Method::GET,
            "/t/shop-b/customers",
            None,
            Some(&a.owner)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&a.app, Method::GET, "/t/shop-a/customers", None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}
