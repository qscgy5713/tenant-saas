use crate::common;

use crate::common::{app, call, register, test_config};
use axum::http::{Method, StatusCode};
use serde_json::json;
use sqlx::PgPool;
use tenant_saas::config::Config;

#[sqlx::test]
async fn register_then_me(pool: PgPool) {
    let app = app(pool);
    let (status, body) = register(&app, "a@example.com", "password123").await;
    assert_eq!(status, StatusCode::CREATED);
    let token = body["token"].as_str().unwrap();
    assert!(body["user"].get("password_hash").is_none());

    let (status, me) = call(&app, Method::GET, "/auth/me", None, Some(token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["email"], "a@example.com");
}

#[sqlx::test]
async fn duplicate_email_is_case_insensitive(pool: PgPool) {
    let app = app(pool);
    assert_eq!(
        register(&app, "a@example.com", "password123").await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        register(&app, "A@Example.com", "password123").await.0,
        StatusCode::CONFLICT
    );
}

#[sqlx::test]
async fn register_validation(pool: PgPool) {
    let app = app(pool);
    assert_eq!(
        register(&app, "not-an-email", "password123").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        register(&app, "a@example.com", "short").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        register(&app, "a@@example.com", "password123").await.0,
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test]
async fn login_success_and_failures(pool: PgPool) {
    let app = app(pool);
    register(&app, "a@example.com", "password123").await;

    let login = |email: &'static str, password: &'static str| {
        let app = app.clone();
        async move {
            call(
                &app,
                Method::POST,
                "/auth/login",
                Some(json!({"email": email, "password": password})),
                None,
            )
            .await
        }
    };

    let (status, body) = login("a@example.com", "password123").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["token"].is_string());

    // Email 不分大小寫
    assert_eq!(
        login("A@EXAMPLE.com", "password123").await.0,
        StatusCode::OK
    );
    // 密碼錯誤與帳號不存在,回應必須相同,避免洩漏哪些 Email 已註冊
    let wrong_pw = login("a@example.com", "wrong-password").await;
    let no_user = login("nobody@example.com", "password123").await;
    assert_eq!(wrong_pw.0, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_pw, no_user);
}

#[sqlx::test]
async fn me_rejects_bad_tokens(pool: PgPool) {
    let app = app(pool);
    assert_eq!(
        call(&app, Method::GET, "/auth/me", None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, Method::GET, "/auth/me", None, Some("garbage"))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );

    // 用不同密鑰簽發的 token 必須被拒絕
    let other = Config {
        jwt_secret: "another-secret-another-secret-12345".into(),
        ..test_config(3600)
    };
    let forged = tenant_saas::auth::JwtKeys::new(&other)
        .issue(uuid::Uuid::new_v4())
        .unwrap();
    assert_eq!(
        call(&app, Method::GET, "/auth/me", None, Some(&forged))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test]
async fn expired_token_is_rejected(pool: PgPool) {
    let app = app(pool.clone());
    let (_, body) = register(&app, "a@example.com", "password123").await;
    let id: uuid::Uuid = body["user"]["id"].as_str().unwrap().parse().unwrap();

    // TTL = 0 簽出的 token 一簽發就過期(jsonwebtoken 預設容忍 60 秒,所以改用過去的時間)
    let keys = tenant_saas::auth::JwtKeys::new(&test_config(0));
    let token = keys
        .issue_at(id, jsonwebtoken::get_current_timestamp() - 3600)
        .unwrap();
    assert_eq!(
        call(&app, Method::GET, "/auth/me", None, Some(&token))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test]
async fn login_and_register_are_rate_limited_per_client(pool: PgPool) {
    let app = common::app_with_auth_limit(pool, 3);
    register(&app, "a@example.com", "password123").await; // 第 1 次(註冊也算)

    // 第 2、3 次:密碼錯誤,正常回 401
    for _ in 0..2 {
        let (status, _) = call(
            &app,
            Method::POST,
            "/auth/login",
            Some(json!({"email": "a@example.com", "password": "wrong-password"})),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    // 超過上限:連正確密碼也被擋(這就是限流的目的:不讓攻擊者無限猜)
    let (status, _) = call(
        &app,
        Method::POST,
        "/auth/login",
        Some(json!({"email": "a@example.com", "password": "password123"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        register(&app, "b@example.com", "password123").await.0,
        StatusCode::TOO_MANY_REQUESTS
    );

    // 已登入後的其他端點不受影響
    assert_eq!(
        call(&app, Method::GET, "/health", None, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, Method::GET, "/auth/me", None, None).await.0,
        StatusCode::UNAUTHORIZED,
        "me 沒被限流(回 401 而非 429)"
    );
}
