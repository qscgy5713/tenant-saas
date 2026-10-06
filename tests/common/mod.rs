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
        auth_rate_limit_per_min: 100_000,
        trust_proxy: false,
        smtp_url: None,
        mail_from: "test <test@example.com>".into(),
        public_base_url: "http://app.test".into(),
        max_shops_per_user: 5,
        // 一般測試註冊後直接開店;驗證流程由 tests/email_verification.rs 用 app_strict 驗證
        require_verified_email: false,
        audit_retention_days: Some(730),
        max_mails_per_recipient_per_hour: 100_000,
        allowed_origins: vec!["http://app.test".into()],
        worker_enabled: false,
        worker_poll_secs: 1,
        metrics_token: None,
        production: false,
        auto_migrate: true,
        stripe: None,
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

/// 以擁有者身分直接切換店家方案(模擬計費流程;店家自己沒有這個權限)
pub async fn set_plan(pool: &PgPool, slug: &str, plan: &str) {
    sqlx::query("UPDATE tenants SET plan_id = $2 WHERE slug = $1")
        .bind(slug)
        .bind(plan)
        .execute(pool)
        .await
        .unwrap();
}

pub fn app_with_auth_limit(pool: PgPool, limit: u32) -> Router {
    let config = Config {
        auth_rate_limit_per_min: limit,
        ..test_config(3600)
    };
    routes::router(AppState::new(pool, &config))
}

/// 最小的 RFC 4180 解析器(測試用):把匯出的檔案還原成列與欄,
/// 才能驗證「引號、逗號、換行」真的來回無損,而不是只比對字串長相
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text
        .strip_prefix('\u{feff}')
        .expect("要有 BOM(Excel 才不會把中文當亂碼)");
    let (mut rows, mut row, mut cell) = (Vec::new(), Vec::new(), String::new());
    let (mut quoted, mut chars) = (false, text.chars().peekable());
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                cell.push('"');
            }
            (true, '"') => quoted = false,
            (true, c) => cell.push(c),
            (false, '"') => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut cell)),
            (false, '\r') if chars.peek() == Some(&'\n') => {
                chars.next();
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            (false, c) => cell.push(c),
        }
    }
    assert!(
        !quoted && cell.is_empty() && row.is_empty(),
        "檔案要以 CRLF 結尾、引號要成對"
    );
    rows
}

/// GET 並回傳原始內容(非 JSON 的回應,例如 CSV)
pub async fn raw_get(
    app: &Router,
    token: Option<&str>,
    uri: &str,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let mut req = Request::builder().method(Method::GET).uri(uri);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}
