mod common;

use axum::http::{Method, StatusCode};
use common::{app, call, register};
use serde_json::{Value, json};
use sqlx::PgPool;
use tenant_saas::{
    mail::{Mailer, MemoryMailer},
    worker::tick,
};

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

async fn fail_times(app: &axum::Router, email: &str, n: u32) {
    for _ in 0..n {
        assert_eq!(
            login(app, email, "wrong-password").await.0,
            StatusCode::UNAUTHORIZED
        );
    }
}

async fn setup(pool: PgPool) -> axum::Router {
    let app = app(pool);
    register(&app, "a@example.com", "password123").await;
    app
}

async fn deliver(pool: &PgPool) -> Vec<(String, String)> {
    let memory = MemoryMailer::new();
    tick(pool, &Mailer::Memory(memory.clone()), "http://app.test")
        .await
        .unwrap();
    memory
        .sent()
        .into_iter()
        .map(|m| (m.subject, m.to))
        .collect()
}

async fn locked_until(pool: &PgPool) -> Option<chrono::DateTime<chrono::Utc>> {
    sqlx::query_scalar("SELECT locked_until FROM users WHERE email = 'a@example.com'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn fifth_failure_locks_and_correct_password_is_refused(pool: PgPool) {
    let app = setup(pool.clone()).await;

    fail_times(&app, "a@example.com", 4).await;
    assert!(locked_until(&pool).await.is_none(), "4 次還不該鎖");

    fail_times(&app, "a@example.com", 1).await;
    assert!(locked_until(&pool).await.is_some());
    // 鎖定中:連正確密碼都不收,而且回應跟「密碼錯誤」「帳號不存在」完全一樣
    let locked = login(&app, "a@example.com", "password123").await;
    assert_eq!(locked.0, StatusCode::UNAUTHORIZED);
    assert_eq!(locked, login(&app, "a@example.com", "wrong-password").await);
    assert_eq!(
        locked,
        login(&app, "nobody@example.com", "x-password").await
    );
}

#[sqlx::test]
async fn success_resets_the_counter(pool: PgPool) {
    let app = setup(pool.clone()).await;
    fail_times(&app, "a@example.com", 4).await;
    assert_eq!(
        login(&app, "a@example.com", "password123").await.0,
        StatusCode::OK
    );
    // 成功後重新計算:再錯 4 次仍不鎖
    fail_times(&app, "a@example.com", 4).await;
    assert!(locked_until(&pool).await.is_none());
    assert_eq!(
        login(&app, "a@example.com", "password123").await.0,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn lock_expires(pool: PgPool) {
    let app = setup(pool.clone()).await;
    fail_times(&app, "a@example.com", 5).await;
    assert_eq!(
        login(&app, "a@example.com", "password123").await.0,
        StatusCode::UNAUTHORIZED
    );

    sqlx::query("UPDATE users SET locked_until = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        login(&app, "a@example.com", "password123").await.0,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn failures_while_locked_do_not_extend_the_lock(pool: PgPool) {
    let app = setup(pool.clone()).await;
    fail_times(&app, "a@example.com", 5).await;
    let first = locked_until(&pool).await.unwrap();

    fail_times(&app, "a@example.com", 10).await;
    assert_eq!(locked_until(&pool).await.unwrap(), first);
}

#[sqlx::test]
async fn after_expiry_counting_starts_over(pool: PgPool) {
    let app = setup(pool.clone()).await;
    fail_times(&app, "a@example.com", 5).await;
    sqlx::query("UPDATE users SET locked_until = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();

    // 鎖定到期後,要再連錯 5 次才會再鎖,不是一錯就鎖
    fail_times(&app, "a@example.com", 4).await;
    assert!(
        locked_until(&pool)
            .await
            .is_none_or(|t| t < chrono::Utc::now())
    );
    assert_eq!(
        login(&app, "a@example.com", "password123").await.0,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn old_failures_are_forgotten(pool: PgPool) {
    let app = setup(pool.clone()).await;
    fail_times(&app, "a@example.com", 4).await;
    // 上次失敗是 20 分鐘前:這次算第 1 次,不是第 5 次
    sqlx::query("UPDATE users SET last_failed_login_at = now() - interval '20 minutes'")
        .execute(&pool)
        .await
        .unwrap();
    fail_times(&app, "a@example.com", 1).await;
    assert!(locked_until(&pool).await.is_none());
    let n: i32 = sqlx::query_scalar("SELECT failed_logins FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test]
async fn lockout_is_per_account(pool: PgPool) {
    let app = setup(pool.clone()).await;
    register(&app, "b@example.com", "password123").await;
    fail_times(&app, "a@example.com", 5).await;
    assert_eq!(
        login(&app, "b@example.com", "password123").await.0,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn owner_is_notified_once_when_locked(pool: PgPool) {
    let app = setup(pool.clone()).await;
    fail_times(&app, "a@example.com", 4).await;
    assert!(deliver(&pool).await.is_empty(), "鎖定前不寄信");

    fail_times(&app, "a@example.com", 8).await;
    let memory = MemoryMailer::new();
    tick(&pool, &Mailer::Memory(memory.clone()), "http://app.test")
        .await
        .unwrap();
    let sent = memory.sent();
    assert_eq!(sent.len(), 1, "只在觸發鎖定那一次寄一封: {sent:?}");
    assert_eq!(sent[0].to, "a@example.com");
    assert!(sent[0].subject.contains("鎖定"));
    // 信中要告訴使用者怎麼處理:等待或用忘記密碼
    let body = &sent[0].body;
    assert!(
        body.contains("15 分鐘") && body.contains("忘記密碼"),
        "{body}"
    );
}

#[sqlx::test]
async fn password_reset_unlocks(pool: PgPool) {
    let app = setup(pool.clone()).await;
    fail_times(&app, "a@example.com", 5).await;
    // 鎖定的通知信先清掉,免得和重設信混在一起
    deliver(&pool).await;

    // 鎖定不影響「忘記密碼」
    let (status, _) = call(
        &app,
        Method::POST,
        "/auth/forgot-password",
        Some(json!({"email": "a@example.com"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let memory = MemoryMailer::new();
    tick(&pool, &Mailer::Memory(memory.clone()), "http://app.test")
        .await
        .unwrap();
    let body = memory.sent().remove(0).body;
    let token: String = body
        .split("/admin/reset#token=")
        .nth(1)
        .unwrap()
        .chars()
        .take(64)
        .collect();

    let (status, _) = call(
        &app,
        Method::POST,
        "/auth/reset-password",
        Some(json!({"token": token, "password": "new-password-2"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(locked_until(&pool).await.is_none());
    assert_eq!(
        login(&app, "a@example.com", "new-password-2").await.0,
        StatusCode::OK
    );
}

/// 租戶角色(`tenant_app`,SQL injection 時攻擊者拿到的就是它)不能讀鎖定欄位,
/// 也不能呼叫計數函式 —— 否則可以替任何人製造鎖定,或幫自己解鎖
#[sqlx::test]
async fn tenant_app_cannot_read_or_drive_lockout(pool: PgPool) {
    let app = setup(pool.clone()).await;
    fail_times(&app, "a@example.com", 5).await;

    for sql in [
        "SELECT locked_until FROM users",
        "UPDATE users SET locked_until = NULL",
        "SELECT login_succeeded((SELECT id FROM users LIMIT 1))",
        "SELECT login_failed((SELECT id FROM users LIMIT 1), 's', 'b')",
    ] {
        // 每個語句各用一個交易:出錯後交易就不能再用了
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SET LOCAL ROLE tenant_app")
            .execute(&mut *tx)
            .await
            .unwrap();
        let err = sqlx::query(sql).execute(&mut *tx).await.unwrap_err();
        assert_eq!(
            tenant_saas::db::pg_code(&err).as_deref(),
            Some("42501"),
            "{sql} 應該被拒絕(權限不足): {err}"
        );
    }
    assert!(locked_until(&pool).await.is_some());
}

/// 應用程式在鎖定時就不會再記失敗;但「檢查」和「記錄」之間有空隙(同時進來的多個請求),
/// 所以資料庫函式自己也要擋:鎖定中記失敗,不能解鎖、也不能延長
#[sqlx::test]
async fn recording_a_failure_while_locked_changes_nothing(pool: PgPool) {
    let app = setup(pool.clone()).await;
    fail_times(&app, "a@example.com", 5).await;
    let before = locked_until(&pool).await.unwrap();

    let again: bool =
        sqlx::query_scalar("SELECT login_failed((SELECT id FROM users), 'subject', 'body')")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!again);
    assert_eq!(locked_until(&pool).await.unwrap(), before);
}
