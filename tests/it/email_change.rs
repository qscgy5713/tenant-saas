//! 更換 Email(migration 0036):重新輸入密碼 → 確認信寄到新地址 → 點了才換 → 通知舊地址。
use crate::common;

use crate::common::{app, call, signup, user_id};
use axum::{
    Router,
    http::{Method, StatusCode},
};
use serde_json::{Value, json};
use sqlx::PgPool;

async fn request(
    app: &Router,
    token: &str,
    new_email: &str,
    password: &str,
) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/auth/change-email",
        Some(json!({"new_email": new_email, "password": password})),
        Some(token),
    )
    .await
}

async fn confirm(app: &Router, raw: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/auth/confirm-email-change",
        Some(json!({"token": raw})),
        None,
    )
    .await
}

async fn login(app: &Router, email: &str) -> StatusCode {
    call(
        app,
        Method::POST,
        "/auth/login",
        Some(json!({"email": email, "password": "password123"})),
        None,
    )
    .await
    .0
}

/// 寄給這個地址的最新一封信
async fn latest_mail(pool: &PgPool, to: &str) -> Option<(String, String)> {
    sqlx::query_as(
        "SELECT subject, body FROM email_outbox WHERE to_email = $1 ORDER BY created_at DESC, id LIMIT 1",
    )
    .bind(to)
    .fetch_optional(pool)
    .await
    .unwrap()
}

/// 寄給這個地址的最新一封信裡 `#token=` 後面的原始 token
async fn link_token(pool: &PgPool, to: &str) -> String {
    let (_, body) = latest_mail(pool, to).await.expect("有寄信");
    let rest = body.split("#token=").nth(1).expect("信裡有 token 連結");
    rest.split_whitespace().next().unwrap().to_string()
}

async fn email_of(pool: &PgPool, email_like: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM users WHERE email = $1::citext")
        .bind(email_like)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn the_email_changes_only_after_the_new_address_confirms(pool: PgPool) {
    let app = app(pool.clone());
    let token = signup(&app, "old@example.com").await;
    let id = user_id(&pool, "old@example.com").await;
    // 先申請一封重設密碼信寄到舊地址:換完之後這個連結要作廢
    call(
        &app,
        Method::POST,
        "/auth/forgot-password",
        Some(json!({"email": "old@example.com"})),
        None,
    )
    .await;
    let reset = link_token(&pool, "old@example.com").await;

    let (status, body) = request(&app, &token, " New@Example.com ", "password123").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    // 還沒點連結:什麼都沒變
    assert_eq!(email_of(&pool, "old@example.com").await, 1);
    assert_eq!(login(&app, "old@example.com").await, StatusCode::OK);

    let raw = link_token(&pool, "New@Example.com").await;
    let (status, body) = confirm(&app, &raw).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(email_of(&pool, "new@example.com").await, 1);
    assert_eq!(
        login(&app, "old@example.com").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(login(&app, "new@example.com").await, StatusCode::OK);
    // 原本的登入不受影響(換 Email 不代表密碼外洩)
    let (status, me) = call(&app, Method::GET, "/auth/me", None, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["email"], "New@Example.com");
    assert_eq!(me["email_verified"], true);

    // 舊地址收到通知,裡面寫著新地址
    let (subject, notice) = latest_mail(&pool, "old@example.com").await.unwrap();
    assert!(subject.contains("已更換"), "{subject}");
    assert!(notice.contains("New@Example.com"), "{notice}");
    // 寄到舊地址的重設密碼連結作廢
    let (status, _) = call(
        &app,
        Method::POST,
        "/auth/reset-password",
        Some(json!({"token": reset, "password": "attacker-pass-1"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // 帳號活動有紀錄;同一個連結不能再用
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT kind FROM account_events WHERE user_id = $1")
            .bind(id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(kinds.contains(&"email_changed".to_string()), "{kinds:?}");
    assert_eq!(confirm(&app, &raw).await.0, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn requests_need_the_password_and_a_free_valid_new_address(pool: PgPool) {
    let app = app(pool.clone());
    let token = signup(&app, "a@example.com").await;
    signup(&app, "taken@example.com").await;
    let id = user_id(&pool, "a@example.com").await;

    // 密碼錯誤:400(不是 401),而且計入帳號鎖定
    let (status, body) = request(&app, &token, "x@example.com", "wrong-password").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let failed: i32 = sqlx::query_scalar("SELECT failed_logins FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(failed, 1);

    assert_eq!(
        request(&app, &token, "not-an-email", "password123").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(&app, &token, "A@Example.com", "password123")
            .await
            .0,
        StatusCode::BAD_REQUEST,
        "和目前的相同(不分大小寫)"
    );
    assert_eq!(
        request(&app, &token, "TAKEN@example.com", "password123")
            .await
            .0,
        StatusCode::CONFLICT
    );
    // 沒登入
    let (status, _) = call(
        &app,
        Method::POST,
        "/auth/change-email",
        Some(json!({"new_email": "x@example.com", "password": "password123"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // 以上都沒有寄出確認信
    let sent: i64 = sqlx::query_scalar("SELECT count(*) FROM email_changes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(sent, 0);
}

#[sqlx::test]
async fn only_the_newest_link_works_and_requests_are_throttled(pool: PgPool) {
    let app = app(pool.clone());
    let token = signup(&app, "a@example.com").await;

    assert_eq!(
        request(&app, &token, "first@example.com", "password123")
            .await
            .0,
        StatusCode::ACCEPTED
    );
    let first = link_token(&pool, "first@example.com").await;
    assert_eq!(
        request(&app, &token, "second@example.com", "password123")
            .await
            .0,
        StatusCode::ACCEPTED
    );
    let second = link_token(&pool, "second@example.com").await;
    assert_eq!(
        confirm(&app, &first).await.0,
        StatusCode::BAD_REQUEST,
        "舊的申請作廢"
    );
    assert_eq!(email_of(&pool, "first@example.com").await, 0);

    assert_eq!(
        request(&app, &token, "third@example.com", "password123")
            .await
            .0,
        StatusCode::ACCEPTED
    );
    // 每小時 3 次
    assert_eq!(
        request(&app, &token, "fourth@example.com", "password123")
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert!(latest_mail(&pool, "fourth@example.com").await.is_none());
    assert_eq!(confirm(&app, &second).await.0, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn the_new_address_receives_no_more_than_the_hourly_mail_quota(pool: PgPool) {
    // 不能用它對別人的信箱灌信:新地址這小時已收到上限數量的信,就不再寄
    let config = tenant_saas::config::Config {
        max_mails_per_recipient_per_hour: 2,
        ..common::test_config(3600)
    };
    let app =
        tenant_saas::routes::router(tenant_saas::routes::AppState::new(pool.clone(), &config));
    let token = signup(&app, "a@example.com").await;
    for _ in 0..2 {
        sqlx::query(
            "INSERT INTO email_outbox (tenant_id, to_email, subject, body) VALUES (NULL, 'victim@example.com', 's', 'b')",
        )
        .execute(&pool)
        .await
        .unwrap();
    }
    assert_eq!(
        request(&app, &token, "victim@example.com", "password123")
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[sqlx::test]
async fn confirming_fails_cleanly_if_the_address_was_taken_or_the_account_deleted(pool: PgPool) {
    let app = app(pool.clone());
    let a = signup(&app, "a@example.com").await;
    let b = signup(&app, "b@example.com").await;

    // 等待確認的期間,別人用這個地址註冊了
    request(&app, &a, "wanted@example.com", "password123").await;
    let raw = link_token(&pool, "wanted@example.com").await;
    signup(&app, "wanted@example.com").await;
    let (status, body) = confirm(&app, &raw).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(email_of(&pool, "a@example.com").await, 1);

    // 申請之後刪了帳號:舊連結不能讓匿名帳號占住新地址
    request(&app, &b, "later@example.com", "password123").await;
    let raw = link_token(&pool, "later@example.com").await;
    let (status, _) = call(
        &app,
        Method::POST,
        "/auth/delete-account",
        Some(json!({"password": "password123"})),
        Some(&b),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(confirm(&app, &raw).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(email_of(&pool, "later@example.com").await, 0);
}
