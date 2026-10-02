mod common;

use axum::http::{Method, StatusCode};
use common::{app, app_with_auth_limit, call, register, signup, test_config};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::Duration;
use tenant_saas::{
    mail::{Mailer, MemoryMailer},
    worker::tick,
};

/// 信中連結 `…/admin/reset#token=<64 字元>` 的 token
fn token_in(body: &str) -> String {
    let after = body
        .split("/admin/reset#token=")
        .nth(1)
        .unwrap_or_else(|| panic!("信裡沒有重設連結: {body}"));
    after.chars().take(64).collect()
}

async fn forgot(app: &axum::Router, email: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/auth/forgot-password",
        Some(json!({"email": email})),
        None,
    )
    .await
}

async fn reset(app: &axum::Router, token: &str, password: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/auth/reset-password",
        Some(json!({"token": token, "password": password})),
        None,
    )
    .await
}

async fn login(app: &axum::Router, email: &str, password: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/auth/login",
        Some(json!({"email": email, "password": password})),
        None,
    )
    .await
}

/// 寄出佇列裡的信,回傳(收件者, 內容)
async fn deliver(pool: &PgPool) -> Vec<(String, String)> {
    let memory = MemoryMailer::new();
    tick(pool, &Mailer::Memory(memory.clone())).await.unwrap();
    memory.sent().into_iter().map(|m| (m.to, m.body)).collect()
}

#[sqlx::test]
async fn unknown_and_known_emails_get_identical_responses(pool: PgPool) {
    let app = app(pool.clone());
    register(&app, "known@example.com", "old-password-1").await;

    let known = forgot(&app, "known@example.com").await;
    let unknown = forgot(&app, "nobody@example.com").await;
    assert_eq!(known.0, StatusCode::ACCEPTED);
    assert_eq!(known, unknown, "不可洩漏哪些 Email 已註冊");

    // 只有已註冊的 Email 真的會收到信
    let sent = deliver(&pool).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "known@example.com");

    // Email 格式錯誤仍是 400(這不洩漏任何事)
    assert_eq!(
        forgot(&app, "not-an-email").await.0,
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test]
async fn full_flow_and_the_token_is_single_use(pool: PgPool) {
    let app = app(pool.clone());
    register(&app, "a@example.com", "old-password-1").await;
    forgot(&app, "a@example.com").await;
    let token = token_in(&deliver(&pool).await[0].1);

    // 資料庫只存雜湊,而且寄出後信件內容被清掉
    let (stored, body): (String, String) = sqlx::query_as(
        "SELECT (SELECT token_hash FROM password_resets), (SELECT body FROM email_outbox)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_ne!(stored, token);
    assert_eq!(body, "", "寄出後 outbox 不留含 token 的內容");

    assert_eq!(
        reset(&app, &token, "new-password-2").await.0,
        StatusCode::OK
    );
    assert_eq!(
        login(&app, "a@example.com", "new-password-2").await.0,
        StatusCode::OK
    );
    assert_eq!(
        login(&app, "a@example.com", "old-password-1").await.0,
        StatusCode::UNAUTHORIZED,
        "舊密碼不能再用"
    );

    // 同一個 token 不能再用
    assert_eq!(
        reset(&app, &token, "third-password-3").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        login(&app, "a@example.com", "new-password-2").await.0,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn bad_tokens_and_weak_passwords_are_rejected_without_consuming_the_token(pool: PgPool) {
    let app = app(pool.clone());
    register(&app, "a@example.com", "old-password-1").await;
    forgot(&app, "a@example.com").await;
    let token = token_in(&deliver(&pool).await[0].1);

    for bad in [&"0".repeat(64), "short", "", &"x".repeat(300)] {
        assert_eq!(
            reset(&app, bad, "new-password-2").await.0,
            StatusCode::BAD_REQUEST,
            "{bad:.10}"
        );
    }
    // 密碼太短:被擋下,而且 token 還能用(使用者可以重新輸入)
    assert_eq!(
        reset(&app, &token, "short").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        reset(&app, &token, "x".repeat(129).as_str()).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        reset(&app, &token, "new-password-2").await.0,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn expired_tokens_are_rejected(pool: PgPool) {
    let app = app(pool.clone());
    register(&app, "a@example.com", "old-password-1").await;
    forgot(&app, "a@example.com").await;
    let token = token_in(&deliver(&pool).await[0].1);
    sqlx::query("UPDATE password_resets SET expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        reset(&app, &token, "new-password-2").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        login(&app, "a@example.com", "old-password-1").await.0,
        StatusCode::OK,
        "密碼沒被改"
    );
}

#[sqlx::test]
async fn a_new_request_invalidates_nothing_but_using_one_voids_all_outstanding_links(pool: PgPool) {
    let app = app(pool.clone());
    register(&app, "a@example.com", "old-password-1").await;
    forgot(&app, "a@example.com").await;
    forgot(&app, "a@example.com").await;
    let tokens: Vec<String> = deliver(&pool)
        .await
        .iter()
        .map(|(_, b)| token_in(b))
        .collect();
    assert_eq!(tokens.len(), 2);

    // 用其中一個重設 → 另一個也一併作廢(攻擊者手上若有舊連結,不能在你改完密碼後再用)
    assert_eq!(
        reset(&app, &tokens[0], "new-password-2").await.0,
        StatusCode::OK
    );
    assert_eq!(
        reset(&app, &tokens[1], "hacked-password-3").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        login(&app, "a@example.com", "new-password-2").await.0,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn at_most_three_requests_per_hour_per_account(pool: PgPool) {
    let app = app(pool.clone());
    register(&app, "a@example.com", "old-password-1").await;
    for _ in 0..6 {
        // 對外的回應完全相同,不洩漏已經被限制
        assert_eq!(forgot(&app, "a@example.com").await.0, StatusCode::ACCEPTED);
    }
    assert_eq!(
        deliver(&pool).await.len(),
        3,
        "同一個帳號每小時最多寄 3 封,避免被拿來灌爆別人的信箱"
    );
    // 超過上限的申請不會留下有效 token
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM password_resets")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 3);
}

#[sqlx::test]
async fn resetting_the_password_logs_out_old_sessions(pool: PgPool) {
    let app = app(pool.clone());
    let old_session = signup(&app, "a@example.com").await;
    assert_eq!(
        call(&app, Method::GET, "/auth/me", None, Some(&old_session))
            .await
            .0,
        StatusCode::OK
    );

    // JWT 的簽發時間只到秒;等超過一秒,才能分辨「改密碼之前」與「之後」
    tokio::time::sleep(Duration::from_millis(1200)).await;
    forgot(&app, "a@example.com").await;
    let token = token_in(&deliver(&pool).await[0].1);
    assert_eq!(
        reset(&app, &token, "new-password-2").await.0,
        StatusCode::OK
    );

    // 舊的(可能被盜的)登入立刻失效 —— 不只 /auth/me,租戶端點也一樣
    assert_eq!(
        call(&app, Method::GET, "/auth/me", None, Some(&old_session))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, Method::GET, "/tenants", None, Some(&old_session))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );

    // 新登入的可以用
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let (_, body) = login(&app, "a@example.com", "new-password-2").await;
    let new_session = body["token"].as_str().unwrap();
    assert_eq!(
        call(&app, Method::GET, "/auth/me", None, Some(new_session))
            .await
            .0,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn tokens_of_deleted_users_stop_working(pool: PgPool) {
    let app = app(pool.clone());
    let token = signup(&app, "gone@example.com").await;
    assert_eq!(
        call(&app, Method::GET, "/auth/me", None, Some(&token))
            .await
            .0,
        StatusCode::OK
    );
    sqlx::query("DELETE FROM users")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, Method::GET, "/auth/me", None, Some(&token))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test]
async fn reset_emails_belong_to_no_tenant_and_tenants_cannot_see_them(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    common::create_tenant(&app, &owner, "shop-a").await;
    forgot(&app, "a@example.com").await;

    let tenant_null: i64 =
        sqlx::query_scalar("SELECT count(*) FROM email_outbox WHERE tenant_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(tenant_null, 1);

    // 租戶角色(請求內)看不到沒有 tenant_id 的列,也就看不到別人的重設信
    let tenant: uuid::Uuid = sqlx::query_scalar("SELECT id FROM tenants")
        .fetch_one(&pool)
        .await
        .unwrap();
    let uid = common::user_id(&pool, "a@example.com").await;
    let mut tx = tenant_saas::db::begin_scoped(&pool, Some(uid), Some(tenant))
        .await
        .unwrap();
    let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM email_outbox")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(visible, 0);
    drop(tx);

    // 執行階段與租戶角色都不能直接碰 password_resets
    let mut tx = tenant_saas::db::begin_scoped(&pool, Some(uid), Some(tenant))
        .await
        .unwrap();
    let err = sqlx::query("SELECT * FROM password_resets")
        .fetch_all(&mut *tx)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("42501")
    );
}

#[sqlx::test]
async fn forgot_and_reset_are_rate_limited_per_client(pool: PgPool) {
    let app = app_with_auth_limit(pool, 3);
    for _ in 0..3 {
        assert_eq!(forgot(&app, "x@example.com").await.0, StatusCode::ACCEPTED);
    }
    assert_eq!(
        forgot(&app, "x@example.com").await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        reset(&app, &"a".repeat(64), "new-password-2").await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    let _ = test_config(1); // 讓共用設定的匯入不被視為未使用
}
