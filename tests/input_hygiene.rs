//! 文字欄位裡的 NUL 等控制字元:資料庫存不了 NUL(會變成 500 並觸發 5xx 告警),一律要在入口回 400。
mod common;

use axum::http::{Method, StatusCode};
use common::{app, call, create_tenant, signup};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test]
async fn nul_in_text_fields_is_a_400_not_a_500(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "owner@example.com").await;
    assert_eq!(
        create_tenant(&app, &owner, "shop-a").await.0,
        StatusCode::CREATED
    );

    let cases = [
        (
            "/auth/register",
            Method::POST,
            json!({"email": "a@example.com", "password": "password123", "name": "a\u{0}b"}),
            None,
        ),
        (
            "/auth/register",
            Method::POST,
            json!({"email": "a\u{0}@example.com", "password": "password123", "name": "ab"}),
            None,
        ),
        (
            "/auth/login",
            Method::POST,
            json!({"email": "a\u{0}@example.com", "password": "password123"}),
            None,
        ),
        (
            "/auth/forgot-password",
            Method::POST,
            json!({"email": "a\u{0}@example.com"}),
            None,
        ),
        (
            "/tenants",
            Method::POST,
            json!({"slug": "shop-b", "name": "b\u{0}"}),
            Some(owner.as_str()),
        ),
        (
            "/t/shop-a",
            Method::PATCH,
            json!({"name": "b\u{0}"}),
            Some(owner.as_str()),
        ),
        (
            "/t/shop-a/services",
            Method::POST,
            json!({"name": "s\u{0}", "duration_minutes": 30, "price_cents": 0}),
            Some(owner.as_str()),
        ),
        (
            "/t/shop-a/invitations",
            Method::POST,
            json!({"email": "x\u{0}@example.com", "role": "staff"}),
            Some(owner.as_str()),
        ),
        (
            "/t/shop-a/customers?q=%00",
            Method::GET,
            serde_json::Value::Null,
            Some(owner.as_str()),
        ),
    ];
    for (uri, method, body, token) in cases {
        let body = (!body.is_null()).then_some(body);
        let (status, resp) = call(&app, method.clone(), uri, body.clone(), token).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{method} {uri} {body:?} → {resp}"
        );
    }
}
