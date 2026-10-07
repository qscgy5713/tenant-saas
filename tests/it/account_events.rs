//! 帳號層級的安全事件:只有本人看得到、不含 IP / 裝置、會過期。

use crate::common::{app, call, register, signup};
use axum::{
    Router,
    http::{Method, StatusCode},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tenant_saas::worker::retention_sweep;

async fn login(app: &Router, email: &str, password: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/auth/login",
        Some(json!({"email": email, "password": password})),
        None,
    )
    .await
}

async fn events(app: &Router, token: &str) -> Vec<String> {
    let (status, body) = call(app, Method::GET, "/auth/security-events", None, Some(token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap().to_string())
        .collect()
}

#[sqlx::test]
async fn sign_ins_failures_and_lockouts_show_up_newest_first(pool: PgPool) {
    let app = app(pool.clone());
    let token = signup(&app, "a@example.com").await;
    assert_eq!(events(&app, &token).await, ["registered"]);

    assert_eq!(
        login(&app, "a@example.com", "password123").await.0,
        StatusCode::OK
    );
    for _ in 0..5 {
        assert_eq!(
            login(&app, "a@example.com", "wrong-password").await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    // 鎖定期間的嘗試不會再記(不會被灌爆)
    assert_eq!(
        login(&app, "a@example.com", "wrong-password").await.0,
        StatusCode::UNAUTHORIZED
    );
    let list = events(&app, &token).await;
    assert_eq!(
        list,
        [
            "locked",
            "login_failed",
            "login_failed",
            "login_failed",
            "login_failed",
            "login_failed",
            "login",
            "registered"
        ]
    );

    // 登出所有裝置也會記(自己的 token 之後失效,所以用重新登入前先確認事件已寫入)
    let (status, _) = call(&app, Method::POST, "/auth/logout-all", None, Some(&token)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT kind FROM account_events ORDER BY id DESC LIMIT 1")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(kinds, ["logout_all"]);
}

#[sqlx::test]
async fn password_resets_are_recorded(pool: PgPool) {
    let app = app(pool.clone());
    let token = signup(&app, "a@example.com").await;
    let (status, _) = call(
        &app,
        Method::POST,
        "/auth/forgot-password",
        Some(json!({"email": "a@example.com"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let body: String =
        sqlx::query_scalar("SELECT body FROM email_outbox WHERE subject LIKE '%重設%'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let raw: String = body
        .split("#token=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    let (status, _) = call(
        &app,
        Method::POST,
        "/auth/reset-password",
        Some(json!({"token": raw, "password": "new-password-1"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // 重設密碼後舊登入失效,用新密碼登入再看
    let (_, body) = login(&app, "a@example.com", "new-password-1").await;
    let fresh = body["token"].as_str().unwrap();
    let _ = token;
    let list = events(&app, fresh).await;
    assert!(list.contains(&"password_reset".to_string()), "{list:?}");
    assert_eq!(list[0], "login");
}

#[sqlx::test]
async fn only_the_owner_of_the_account_sees_the_events_and_nothing_personal_is_stored(
    pool: PgPool,
) {
    let app = app(pool.clone());
    let a = signup(&app, "a@example.com").await;
    let b = signup(&app, "b@example.com").await;
    login(&app, "a@example.com", "wrong-password").await;
    assert_eq!(events(&app, &b).await, ["registered"], "看不到別人的事件");
    assert_eq!(events(&app, &a).await, ["login_failed", "registered"]);
    // 不認識的 Email 不會留下任何事件
    login(&app, "nobody@example.com", "wrong-password").await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM account_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        3
    );
    // 資料表裡只有 id / 使用者 / 種類 / 時間,沒有 IP 與裝置欄位
    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns WHERE table_name = 'account_events' ORDER BY column_name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(columns, ["created_at", "id", "kind", "user_id"]);
    // 沒登入看不到
    assert_eq!(
        call(&app, Method::GET, "/auth/security-events", None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test]
async fn old_events_are_purged_and_deleting_the_account_removes_them(pool: PgPool) {
    let app = app(pool.clone());
    let token = signup(&app, "a@example.com").await;
    sqlx::query("UPDATE account_events SET created_at = now() - interval '200 days'")
        .execute(&pool)
        .await
        .unwrap();
    login(&app, "a@example.com", "password123").await;
    assert_eq!(
        retention_sweep(&pool, Some(730))
            .await
            .account_events_purged,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM account_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1,
        "近期的留著"
    );
    // 下限
    let err = sqlx::query("SELECT retention_purge_account_events(5, 10)")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("22023")
    );

    let (status, _) = call(
        &app,
        Method::POST,
        "/auth/delete-account",
        Some(json!({"password": "password123"})),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM account_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    // 這個 Email 重新註冊:全新的帳號,從頭開始
    let (status, body) = register(&app, "a@example.com", "password123").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        events(&app, body["token"].as_str().unwrap()).await,
        ["registered"]
    );
}
