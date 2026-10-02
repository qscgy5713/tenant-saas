//! 用「正式環境的受限帳號」把整個流程跑一遍。
//!
//! 一般測試都用超級使用者連線(Docker 的 POSTGRES_USER),超級使用者會繞過權限與 RLS,
//! 所以那些測試**證明不了**正式環境用受限帳號時一切正常。這個測試補上這個缺口。
//!
//! 需要一個乾淨的 Postgres 叢集(角色是叢集共用的),所以預設 `#[ignore]`:
//!   RESTRICTED_RUNTIME_URL=postgres://tenant_runtime:...@host/tenant_saas \
//!   cargo test --test restricted_role -- --ignored
//! 前置步驟(provision → migrate → 啟用 runtime 登入)見 docs/deployment.md 與 .github/workflows/ci.yml。

mod common;

use axum::http::{Method, StatusCode};
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Asia::Taipei;
use common::{app, call, create_service, create_tenant, signup};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use tenant_saas::{
    availability::local_to_utc,
    dbrole,
    mail::{Mailer, MemoryMailer},
    worker::tick,
};
use uuid::Uuid;

fn pg_code(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.to_string())
}

#[tokio::test]
#[ignore = "需要依 docs/deployment.md 佈建好的受限帳號;CI 的 restricted-role 工作會執行"]
async fn whole_app_works_with_the_restricted_runtime_account() {
    let url = std::env::var("RESTRICTED_RUNTIME_URL").expect("請設定 RESTRICTED_RUNTIME_URL");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("以 runtime 帳號連線");
    let unique = Uuid::new_v4().simple().to_string();
    let slug = format!("rr-{}", &unique[..10]);

    // 1. 帳號本身夠受限
    let problems = dbrole::problems(&pool).await.unwrap();
    assert!(problems.is_empty(), "runtime 帳號過大:{problems:?}");

    // 2. 失敗即關閉:忘了切換角色、直接對連線池查詢,必須被擋下
    for sql in [
        "SELECT count(*) FROM services",
        "SELECT count(*) FROM bookings",
        "SELECT count(*) FROM memberships",
        "SELECT count(*) FROM tenants",
        "SELECT count(*) FROM email_outbox",
        "SELECT count(*) FROM audit_logs",
        "UPDATE users SET name = 'x'",
        "DELETE FROM users",
        "UPDATE plans SET max_staff = NULL",
        "CREATE TABLE evil (id int)",
    ] {
        let err = sqlx::query(sql).execute(&pool).await.unwrap_err();
        assert_eq!(
            pg_code(&err).as_deref(),
            Some("42501"),
            "{sql} 應被拒絕,實際:{err}"
        );
    }

    // 3. 整個應用流程(含 SECURITY DEFINER 函式、worker 角色)在受限帳號下都能正常運作
    let app = app(pool.clone());
    let owner = signup(&app, &format!("owner-{unique}@example.com")).await;
    let (status, body) = create_tenant(&app, &owner, &slug).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "建立店家(create_tenant 函式): {body}"
    );

    let svc = create_service(&app, &owner, &slug, "剪髮").await;
    let service_id = svc["id"].as_str().unwrap().to_string();
    call(
        &app,
        Method::PATCH,
        &format!("/t/{slug}/services/{service_id}"),
        Some(json!({"duration_minutes": 60})),
        Some(&owner),
    )
    .await;
    let (_, me) = call(&app, Method::GET, "/auth/me", None, Some(&owner)).await;
    let owner_id = me["id"].as_str().unwrap().to_string();
    let hours: Vec<Value> = (0..7)
        .map(|d| json!({"weekday": d, "start": "09:00", "end": "18:00"}))
        .collect();
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &format!("/t/{slug}/members/{owner_id}/working-hours"),
            Some(json!({"hours": hours})),
            Some(&owner)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &format!("/t/{slug}/members/{owner_id}/services"),
            Some(json!({"service_ids": [service_id]})),
            Some(&owner)
        )
        .await
        .0,
        StatusCode::OK
    );

    // 公開預約:建立申請 → worker 寄信(tenant_worker 角色)→ 顧客用 token 確認(booking_tenant_by_token 函式)
    let day = Utc::now().with_timezone(&Taipei).date_naive() + Duration::days(3);
    let start: DateTime<Utc> = local_to_utc(
        Taipei,
        day,
        chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
    )
    .unwrap();
    let customer = format!("customer-{unique}@example.com");
    let (status, req) = call(&app, Method::POST, &format!("/public/shops/{slug}/bookings"), Some(json!({
        "service_id": service_id, "start": start.to_rfc3339(), "customer": {"name": "顧客", "email": customer},
    })), None).await;
    assert_eq!(status, StatusCode::CREATED, "{req}");

    let memory = MemoryMailer::new();
    let stats = tick(&pool, &Mailer::Memory(memory.clone())).await.unwrap();
    assert!(stats.sent >= 1, "worker 要能在受限帳號下寄信: {stats:?}");
    let mail = memory
        .sent()
        .into_iter()
        .find(|m| m.to == customer)
        .expect("找到驗證信");
    let token: String = mail
        .body
        .split("/bookings/")
        .nth(1)
        .unwrap()
        .chars()
        .take(64)
        .collect();

    let (status, view) = call(
        &app,
        Method::GET,
        &format!("/public/bookings/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "用 token 查看預約(booking_tenant_by_token): {view}"
    );
    let (status, view) = call(
        &app,
        Method::POST,
        &format!("/public/bookings/{token}/confirm"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["status"], "confirmed");
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/public/bookings/{token}/cancel"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 邀請與接受(accept_invitation 函式)
    let invitee_email = format!("invitee-{unique}@example.com");
    let invitee = signup(&app, &invitee_email).await;
    let (_, inv) = call(
        &app,
        Method::POST,
        &format!("/t/{slug}/invitations"),
        Some(json!({"email": invitee_email, "role": "staff"})),
        Some(&owner),
    )
    .await;
    let (status, accepted) = call(
        &app,
        Method::POST,
        "/invitations/accept",
        Some(json!({"token": inv["token"]})),
        Some(&invitee),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(
        call(
            &app,
            Method::GET,
            &format!("/t/{slug}/me"),
            None,
            Some(&invitee)
        )
        .await
        .0,
        StatusCode::OK
    );

    // 稽核、方案、用量、員工端預約
    let (status, audit) = call(
        &app,
        Method::GET,
        &format!("/t/{slug}/audit-logs?limit=100"),
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let actions: Vec<&str> = audit["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    for expected in [
        "tenant.created",
        "booking.requested",
        "booking.confirmed",
        "booking.cancelled",
        "invitation.accepted",
    ] {
        assert!(
            actions.contains(&expected),
            "稽核缺少 {expected}: {actions:?}"
        );
    }
    assert_eq!(
        call(
            &app,
            Method::GET,
            &format!("/t/{slug}/plan"),
            None,
            Some(&owner)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, Method::GET, "/plans", None, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            Method::GET,
            &format!("/t/{slug}/bookings"),
            None,
            Some(&owner)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, Method::GET, "/health", None, None).await.0,
        StatusCode::OK
    );

    // 4. 隔離在受限帳號下仍然成立:別家店的人看不到這家店
    let other = signup(&app, &format!("other-{unique}@example.com")).await;
    assert_eq!(
        call(
            &app,
            Method::GET,
            &format!("/t/{slug}/audit-logs"),
            None,
            Some(&other)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            Method::GET,
            &format!("/t/{slug}/services"),
            None,
            Some(&other)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}
