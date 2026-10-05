//! 每位使用者可擁有的店家數量上限。

mod common;

use axum::{Router, http::Method, http::StatusCode};
use common::{add_member, app, call, create_tenant, signup, test_config};
use serde_json::json;
use sqlx::PgPool;
use tenant_saas::{
    config::Config,
    routes::{self, AppState},
};

fn app_with_limit(pool: PgPool, limit: u32) -> Router {
    let config = Config {
        max_shops_per_user: limit,
        ..test_config(3600)
    };
    routes::router(AppState::new(pool, &config))
}

async fn shop_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM tenants")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn the_limit_applies_per_owner_and_tells_you_why(pool: PgPool) {
    let app = app_with_limit(pool.clone(), 2);
    let a = signup(&app, "a@example.com").await;
    let b = signup(&app, "b@example.com").await;

    assert_eq!(
        create_tenant(&app, &a, "a-one").await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        create_tenant(&app, &a, "a-two").await.0,
        StatusCode::CREATED
    );
    let (status, body) = create_tenant(&app, &a, "a-three").await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("最多可以建立 2 家店"),
        "{body}"
    );
    assert_eq!(shop_count(&pool).await, 2, "被拒絕的不能留下半成品");
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = 'tenant.created'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 2);

    // 別人不受影響
    assert_eq!(
        create_tenant(&app, &b, "b-one").await.0,
        StatusCode::CREATED
    );
    // 被拒絕的代稱沒有被占用
    assert_eq!(
        create_tenant(&app, &b, "a-three").await.0,
        StatusCode::CREATED
    );
}

#[sqlx::test]
async fn shops_you_were_invited_to_do_not_count(pool: PgPool) {
    let app = app_with_limit(pool.clone(), 1);
    let owner = signup(&app, "o@example.com").await;
    let me = signup(&app, "me@example.com").await;
    create_tenant(&app, &owner, "theirs").await;
    add_member(&pool, "theirs", "me@example.com", "staff").await;

    // 我是別人店裡的員工,自己的名額還是完整的
    assert_eq!(
        create_tenant(&app, &me, "mine").await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        create_tenant(&app, &me, "mine-2").await.0,
        StatusCode::PAYMENT_REQUIRED
    );
}

#[sqlx::test]
async fn simultaneous_requests_cannot_slip_past_the_limit(pool: PgPool) {
    let app = app_with_limit(pool.clone(), 3);
    let token = signup(&app, "a@example.com").await;

    let tasks: Vec<_> = (0..10)
        .map(|i| {
            let (app, token) = (app.clone(), token.clone());
            tokio::spawn(async move {
                call(
                    &app,
                    Method::POST,
                    "/tenants",
                    Some(json!({"slug": format!("race-{i}"), "name": "店"})),
                    Some(&token),
                )
                .await
                .0
            })
        })
        .collect();
    let mut statuses = Vec::new();
    for t in tasks {
        statuses.push(t.await.unwrap());
    }
    let created = statuses
        .iter()
        .filter(|s| **s == StatusCode::CREATED)
        .count();
    let refused = statuses
        .iter()
        .filter(|s| **s == StatusCode::PAYMENT_REQUIRED)
        .count();
    assert_eq!((created, refused), (3, 7), "{statuses:?}");
    assert_eq!(shop_count(&pool).await, 3);
}

/// 舊的三參數版本如果還在,tenant_app 可以直接呼叫它而繞過上限
#[sqlx::test]
async fn the_old_unlimited_function_is_gone(pool: PgPool) {
    let app = app(pool.clone());
    signup(&app, "a@example.com").await;
    let user: uuid::Uuid = sqlx::query_scalar("SELECT id FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut tx = tenant_saas::db::begin_scoped(&pool, Some(user), None)
        .await
        .unwrap();
    let err = sqlx::query("SELECT create_tenant('sneaky', 'x', 'Asia/Taipei')")
        .execute(&mut *tx)
        .await
        .unwrap_err();
    assert_eq!(
        tenant_saas::db::pg_code(&err).as_deref(),
        Some("42883"),
        "應該是「函式不存在」: {err}"
    );
}

/// 確定性地驗證鎖:第一個建店交易還沒提交時,同一位使用者的第二個建店必須等它。
/// (沒有鎖的話,第二個看不到第一個尚未提交的店,會立刻通過檢查 —— 上面「同時 10 個請求」
/// 的測試靠時間碰運氣,抓不到這個。)
#[sqlx::test]
async fn a_second_creation_waits_for_the_first_to_finish(pool: PgPool) {
    let app = app(pool.clone());
    signup(&app, "a@example.com").await;
    let user: uuid::Uuid = sqlx::query_scalar("SELECT id FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();

    let mut first = tenant_saas::db::begin_scoped(&pool, Some(user), None)
        .await
        .unwrap();
    sqlx::query("SELECT create_tenant('first', 'x', 'Asia/Taipei', 1)")
        .execute(&mut *first)
        .await
        .unwrap();

    let pool2 = pool.clone();
    let second = tokio::spawn(async move {
        let mut tx = tenant_saas::db::begin_scoped(&pool2, Some(user), None)
            .await
            .unwrap();
        sqlx::query("SELECT create_tenant('second', 'x', 'Asia/Taipei', 1)")
            .execute(&mut *tx)
            .await
            .map(|_| ())
    });

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(!second.is_finished(), "第二個建店應該卡在等第一個的鎖");

    first.commit().await.unwrap();
    let err = second.await.unwrap().unwrap_err();
    assert_eq!(
        tenant_saas::db::pg_code(&err).as_deref(),
        Some("53400"),
        "第一個提交後,第二個應該看到已達上限: {err}"
    );
    assert_eq!(shop_count(&pool).await, 1);
}
