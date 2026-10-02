#![allow(dead_code)]

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tenant_saas::{
    config::Config,
    routes::{self, AppState},
};
use tower::ServiceExt;

pub fn test_config(ttl: u64) -> Config {
    Config {
        database_url: String::new(),
        bind_addr: String::new(),
        jwt_secret: "test-secret-test-secret-test-secret-123".into(),
        jwt_ttl_secs: ttl,
        public_rate_limit_per_min: 100_000,
        trust_proxy: false,
        smtp_url: None,
        mail_from: "test <test@example.com>".into(),
        public_base_url: "http://app.test".into(),
        worker_enabled: false,
        worker_poll_secs: 1,
    }
}

pub fn app(pool: PgPool) -> Router {
    routes::router(AppState::new(pool, &test_config(3600)))
}

pub async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

pub async fn register(app: &Router, email: &str, password: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/auth/register",
        Some(json!({"email": email, "password": password, "name": "小明"})),
        None,
    )
    .await
}

/// 註冊並回傳 token
pub async fn signup(app: &Router, email: &str) -> String {
    let (status, body) = register(app, email, "password123").await;
    assert_eq!(status, StatusCode::CREATED);
    body["token"].as_str().unwrap().to_string()
}

pub async fn create_tenant(app: &Router, token: &str, slug: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/tenants",
        Some(json!({"slug": slug, "name": format!("店 {slug}")})),
        Some(token),
    )
    .await
}

/// 以擁有者身分直接把使用者加進店家(邀請流程在 M5 才實作)
pub async fn add_member(pool: &PgPool, slug: &str, email: &str, role: &str) -> uuid::Uuid {
    sqlx::query_scalar(
        "INSERT INTO memberships (tenant_id, user_id, role)
         SELECT t.id, u.id, $3::member_role FROM tenants t, users u
         WHERE t.slug = $1 AND u.email = $2
         RETURNING user_id",
    )
    .bind(slug)
    .bind(email)
    .bind(role)
    .fetch_one(pool)
    .await
    .unwrap()
}

pub async fn user_id(pool: &PgPool, email: &str) -> uuid::Uuid {
    sqlx::query_scalar("SELECT id FROM users WHERE email = $1::citext")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

pub async fn create_service(app: &Router, token: &str, slug: &str, name: &str) -> Value {
    let (status, body) = call(
        app,
        Method::POST,
        &format!("/t/{slug}/services"),
        Some(json!({"name": name, "duration_minutes": 30, "price_cents": 500})),
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

pub fn app_with_rate_limit(pool: PgPool, limit: u32) -> Router {
    let config = Config {
        public_rate_limit_per_min: limit,
        ..test_config(3600)
    };
    routes::router(AppState::new(pool, &config))
}
