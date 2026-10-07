//! 店家資訊(簡介、地址、電話):只有店主能改、驗證、顯示在公開預約頁。

use crate::common::{add_member, app, call, create_tenant, signup};
use axum::{
    Router,
    http::{Method, StatusCode},
};
use serde_json::{Value, json};
use sqlx::PgPool;

async fn put(app: &Router, token: &str, body: Value) -> (StatusCode, Value) {
    call(
        app,
        Method::PUT,
        "/t/shop-a/profile",
        Some(body),
        Some(token),
    )
    .await
}

async fn public(app: &Router) -> Value {
    call(app, Method::GET, "/public/shops/shop-a", None, None)
        .await
        .1
}

async fn setup(pool: &PgPool) -> (Router, String) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    (app, owner)
}

#[sqlx::test]
async fn the_owner_sets_the_profile_and_customers_see_it(pool: PgPool) {
    let (app, owner) = setup(&pool).await;
    // 一開始都是空的
    let shop = public(&app).await;
    assert!(shop["description"].is_null() && shop["address"].is_null() && shop["phone"].is_null());

    let (status, body) = put(
        &app,
        &owner,
        json!({
            "description": "  專做自然捲的小店。\n歡迎預約!  ",
            "address": "台北市中山區南京東路一段 1 號",
            "phone": "+886 2-1234-5678 #12"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["description"], "專做自然捲的小店。\n歡迎預約!"); // 去頭尾空白,簡介保留換行
    let shop = public(&app).await;
    assert_eq!(shop["address"], "台北市中山區南京東路一段 1 號");
    assert_eq!(shop["phone"], "+886 2-1234-5678 #12");

    // 整組取代:沒帶 / 空字串 = 清除
    let (status, body) = put(&app, &owner, json!({"address": "新地址", "phone": ""})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let shop = public(&app).await;
    assert!(shop["description"].is_null() && shop["phone"].is_null());
    assert_eq!(shop["address"], "新地址");

    // 稽核只記哪些欄位變了,不記內容
    let details: Vec<Value> = sqlx::query_scalar(
        "SELECT detail FROM audit_logs WHERE action = 'tenant.profile_updated' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        details[0],
        json!({"changed": ["description", "address", "phone"]})
    );
    assert_eq!(
        details[1],
        json!({"changed": ["description", "address", "phone"]})
    );
    assert!(!details[0].to_string().contains("南京"));

    // 只改一個欄位 / 完全沒變:稽核只列真的變了的
    put(
        &app,
        &owner,
        json!({"address": "新地址", "phone": "02-1111-2222"}),
    )
    .await;
    put(
        &app,
        &owner,
        json!({"address": "新地址", "phone": "02-1111-2222"}),
    )
    .await;
    let details: Vec<Value> = sqlx::query_scalar(
        "SELECT detail FROM audit_logs WHERE action = 'tenant.profile_updated' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(details[2], json!({"changed": ["phone"]}));
    assert_eq!(details[3], json!({"changed": []}));
}

#[sqlx::test]
async fn only_the_owner_may_change_it(pool: PgPool) {
    let (app, owner) = setup(&pool).await;
    let manager = signup(&app, "m@example.com").await;
    let staff = signup(&app, "s@example.com").await;
    add_member(&pool, "shop-a", "m@example.com", "manager").await;
    add_member(&pool, "shop-a", "s@example.com", "staff").await;
    assert_eq!(
        put(&app, &manager, json!({"address": "x"})).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        put(&app, &staff, json!({"address": "x"})).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, _) = call(
        &app,
        Method::PUT,
        "/t/shop-a/profile",
        Some(json!({})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(public(&app).await["address"].is_null());
    let _ = owner;
}

#[sqlx::test]
async fn invalid_values_are_rejected(pool: PgPool) {
    let (app, owner) = setup(&pool).await;
    let bad = |body: Value| {
        let (app, owner) = (app.clone(), owner.clone());
        async move { put(&app, &owner, body).await.0 }
    };
    assert_eq!(
        bad(json!({"description": "字".repeat(501)})).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        bad(json!({"address": "字".repeat(201)})).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        bad(json!({"phone": "1".repeat(31)})).await,
        StatusCode::BAD_REQUEST
    );
    // 地址 / 電話不能有換行;所有欄位不能有其他控制字元
    assert_eq!(
        bad(json!({"address": "第一行\n第二行"})).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        bad(json!({"description": "含\u{0}空字元"})).await,
        StatusCode::BAD_REQUEST
    );
    // 電話會做成 tel: 連結:只允許數字與 + - ( ) # 空白
    assert_eq!(
        bad(json!({"phone": "javascript:alert(1)"})).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        bad(json!({"phone": "0912-345-678 ext.5"})).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        bad(json!({"phone": "02) 12<script>"})).await,
        StatusCode::BAD_REQUEST
    );
    // 邊界值可以
    assert_eq!(bad(json!({"description": "字".repeat(500), "address": "字".repeat(200), "phone": "1".repeat(30)})).await, StatusCode::OK);
    // 全部沒通過驗證的,不會留下任何稽核
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_logs WHERE action = 'tenant.profile_updated'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
}

#[sqlx::test]
async fn the_database_enforces_the_lengths_too(pool: PgPool) {
    let (_app, _owner) = setup(&pool).await;
    let err = sqlx::query("UPDATE tenants SET description = repeat('x', 501)")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23514")
    );
    let err = sqlx::query("UPDATE tenants SET phone = ''")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23514")
    );
}
