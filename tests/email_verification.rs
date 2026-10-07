mod common;

use axum::{
    Router,
    http::{Method, StatusCode},
};
use common::{call, create_tenant, register, test_config};
use serde_json::{Value, json};
use sqlx::PgPool;
use tenant_saas::{
    config::Config,
    routes::{self, AppState},
};

/// 要求驗證 Email 的應用程式(一般測試把它關掉,見 tests/common)
fn strict_app(pool: PgPool) -> Router {
    let config = Config {
        require_verified_email: true,
        ..test_config(3600)
    };
    routes::router(AppState::new(pool, &config))
}

async fn login(app: &Router, email: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/auth/login",
        Some(json!({"email": email, "password": "password123"})),
        None,
    )
    .await
}

async fn me(app: &Router, token: &str) -> (StatusCode, Value) {
    call(app, Method::GET, "/auth/me", None, Some(token)).await
}

async fn verify(app: &Router, token: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/auth/verify-email",
        Some(json!({"token": token})),
        None,
    )
    .await
}

async fn resend(app: &Router, token: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/auth/resend-verification",
        None,
        Some(token),
    )
    .await
}

/// 寄給這個 Email 的最新一封信裡 `#token=` 後面的原始 token
async fn latest_token(pool: &PgPool, email: &str, subject: &str) -> String {
    let body: String = sqlx::query_scalar(
        "SELECT body FROM email_outbox WHERE to_email = $1 AND subject LIKE $2
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(email)
    .bind(format!("%{subject}%"))
    .fetch_one(pool)
    .await
    .unwrap();
    let rest = body.split("#token=").nth(1).expect("信裡有 token 連結");
    rest.split_whitespace().next().unwrap().to_string()
}

async fn mail_count(pool: &PgPool, email: &str, subject: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM email_outbox WHERE to_email = $1 AND subject LIKE $2")
        .bind(email)
        .bind(format!("%{subject}%"))
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn signup_unverified(app: &Router, email: &str) -> String {
    let (status, body) = register(app, email, "password123").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["user"]["email_verified"], false);
    body["token"].as_str().unwrap().to_string()
}

#[sqlx::test]
async fn unverified_users_cannot_open_a_shop_until_they_click_the_link(pool: PgPool) {
    let app = strict_app(pool.clone());
    let token = signup_unverified(&app, "a@example.com").await;

    // 註冊時寄出驗證信;沒驗證不能開店,錯誤訊息要說清楚怎麼辦
    assert_eq!(mail_count(&pool, "a@example.com", "驗證").await, 1);
    let (status, body) = create_tenant(&app, &token, "shop-a").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body["error"].as_str().unwrap().contains("驗證 Email"),
        "{body}"
    );
    assert_eq!(me(&app, &token).await.1["email_verified"], false);
    // 其他功能不受影響:登入、看自己的店家清單
    assert_eq!(login(&app, "a@example.com").await.0, StatusCode::OK);
    assert_eq!(
        call(&app, Method::GET, "/tenants", None, Some(&token))
            .await
            .0,
        StatusCode::OK
    );

    // 點連結(不需要登入:常常在另一台裝置開信)
    let raw = latest_token(&pool, "a@example.com", "驗證").await;
    assert_eq!(verify(&app, &raw).await.0, StatusCode::OK);
    assert_eq!(me(&app, &token).await.1["email_verified"], true);
    // 驗證成功會記在帳號活動裡
    let (_, events) = call(
        &app,
        Method::GET,
        "/auth/security-events",
        None,
        Some(&token),
    )
    .await;
    assert_eq!(events[0]["kind"], "email_verified", "{events}");
    assert_eq!(
        create_tenant(&app, &token, "shop-a").await.0,
        StatusCode::CREATED
    );

    // 連結只能用一次;亂給的 token 也是同一個 400
    assert_eq!(verify(&app, &raw).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        verify(&app, "x".repeat(40).as_str()).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(verify(&app, "short").await.0, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn the_token_is_only_stored_hashed_and_expires(pool: PgPool) {
    let app = strict_app(pool.clone());
    signup_unverified(&app, "a@example.com").await;
    let raw = latest_token(&pool, "a@example.com", "驗證").await;
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM email_verifications")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(stored, raw);
    assert_eq!(stored, tenant_saas::token::hash(&raw));

    sqlx::query("UPDATE email_verifications SET expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(verify(&app, &raw).await.0, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn resending_is_throttled_and_a_new_link_invalidates_nothing_but_works(pool: PgPool) {
    let app = strict_app(pool.clone());
    let token = signup_unverified(&app, "a@example.com").await;

    // 註冊那封算第 1 封;每小時最多 3 封
    assert_eq!(resend(&app, &token).await.0, StatusCode::ACCEPTED);
    assert_eq!(resend(&app, &token).await.0, StatusCode::ACCEPTED);
    assert_eq!(resend(&app, &token).await.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(mail_count(&pool, "a@example.com", "驗證").await, 3);

    // 用最新那封驗證
    let raw = latest_token(&pool, "a@example.com", "驗證").await;
    assert_eq!(verify(&app, &raw).await.0, StatusCode::OK);
    // 已驗證:不再寄,回 409,而且不多寄信
    assert_eq!(resend(&app, &token).await.0, StatusCode::CONFLICT);
    assert_eq!(mail_count(&pool, "a@example.com", "驗證").await, 3);
    // 同一個人先前寄出的其他連結也一併作廢
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM email_verifications WHERE used_at IS NULL"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
}

#[sqlx::test]
async fn resending_requires_login(pool: PgPool) {
    let app = strict_app(pool);
    let (status, _) = call(&app, Method::POST, "/auth/resend-verification", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn resetting_the_password_also_proves_the_mailbox(pool: PgPool) {
    let app = strict_app(pool.clone());
    let token = signup_unverified(&app, "a@example.com").await;
    let (status, _) = call(
        &app,
        Method::POST,
        "/auth/forgot-password",
        Some(json!({"email": "a@example.com"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let raw = latest_token(&pool, "a@example.com", "重設").await;
    let (status, body) = call(
        &app,
        Method::POST,
        "/auth/reset-password",
        Some(json!({"token": raw, "password": "new-password-1"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let _ = token;
    let (status, body) = call(
        &app,
        Method::POST,
        "/auth/login",
        Some(json!({"email": "a@example.com", "password": "new-password-1"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user"]["email_verified"], true);
}

#[sqlx::test]
async fn accepting_an_invitation_proves_the_mailbox(pool: PgPool) {
    let app = strict_app(pool.clone());
    let owner = signup_unverified(&app, "a@example.com").await;
    let raw = latest_token(&pool, "a@example.com", "驗證").await;
    assert_eq!(verify(&app, &raw).await.0, StatusCode::OK);
    assert_eq!(
        create_tenant(&app, &owner, "shop-a").await.0,
        StatusCode::CREATED
    );

    let (status, inv) = call(
        &app,
        Method::POST,
        "/t/shop-a/invitations",
        Some(json!({"email": "b@example.com", "role": "staff"})),
        Some(&owner),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{inv}");
    let invitee = signup_unverified(&app, "b@example.com").await;
    let (status, body) = call(
        &app,
        Method::POST,
        "/invitations/accept",
        Some(json!({"token": inv["token"]})),
        Some(&invitee),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(me(&app, &invitee).await.1["email_verified"], true);
}

#[sqlx::test]
async fn logging_out_everywhere_revokes_every_existing_token(pool: PgPool) {
    let app = strict_app(pool.clone());
    let first = signup_unverified(&app, "a@example.com").await;
    // 簽發時間只到秒:後面的登入要和撤銷分在不同秒,不然會被當成「撤銷之前」
    let (_, second) = login(&app, "a@example.com").await;
    let second = second["token"].as_str().unwrap().to_string();
    assert_eq!(me(&app, &first).await.0, StatusCode::OK);
    assert_eq!(me(&app, &second).await.0, StatusCode::OK);

    let (status, _) = call(&app, Method::POST, "/auth/logout-all", None, Some(&first)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(me(&app, &first).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(me(&app, &second).await.0, StatusCode::UNAUTHORIZED);

    // 之後重新登入拿到的新 token 可以用(等到下一秒,避開同一秒的窗口)
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (status, fresh) = login(&app, "a@example.com").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        me(&app, fresh["token"].as_str().unwrap()).await.0,
        StatusCode::OK
    );
    // 沒登入不能呼叫
    let (status, _) = call(&app, Method::POST, "/auth/logout-all", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn logging_out_everywhere_only_affects_that_user(pool: PgPool) {
    let app = strict_app(pool.clone());
    let a = signup_unverified(&app, "a@example.com").await;
    let b = signup_unverified(&app, "b@example.com").await;
    call(&app, Method::POST, "/auth/logout-all", None, Some(&a)).await;
    assert_eq!(me(&app, &a).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(me(&app, &b).await.0, StatusCode::OK);
}

#[sqlx::test]
async fn when_verification_is_not_required_new_users_count_as_verified(pool: PgPool) {
    // 關掉時(只有開發 / 測試能關):新使用者直接視為已驗證、不寄驗證信,註冊後可以直接開店
    let app = routes::router(AppState::new(pool.clone(), &test_config(3600)));
    let (status, body) = register(&app, "a@example.com", "password123").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["user"]["email_verified"], true);
    let token = body["token"].as_str().unwrap();
    assert_eq!(mail_count(&pool, "a@example.com", "驗證").await, 0);
    assert_eq!(
        create_tenant(&app, token, "shop-a").await.0,
        StatusCode::CREATED
    );
}
