//! 使用者帳號刪除(本人要求):匿名化、不能刪的情況。

use crate::common::{
    add_member, app, call, create_service, create_tenant, register, signup, user_id,
};
use axum::{
    Router,
    http::{Method, StatusCode},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Asia::Taipei;
use serde_json::{Value, json};
use sqlx::PgPool;
use tenant_saas::availability::local_to_utc;
use uuid::Uuid;

fn day(offset: i64) -> NaiveDate {
    Utc::now().with_timezone(&Taipei).date_naive() + Duration::days(offset)
}

fn at(date: NaiveDate, h: u32) -> DateTime<Utc> {
    local_to_utc(
        Taipei,
        date,
        chrono::NaiveTime::from_hms_opt(h, 0, 0).unwrap(),
    )
    .unwrap()
}

struct Shop {
    app: Router,
    owner: String,
    staff: String,
    staff_id: Uuid,
    service_id: String,
}

/// 店主 a、員工 b(每天營業、提供服務)
async fn shop(pool: &PgPool) -> Shop {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    assert_eq!(
        create_tenant(&app, &owner, "shop-a").await.0,
        StatusCode::CREATED
    );
    let staff = signup(&app, "b@example.com").await;
    add_member(pool, "shop-a", "b@example.com", "staff").await;
    let staff_id = user_id(pool, "b@example.com").await;
    let service = create_service(&app, &owner, "shop-a", "剪髮").await;
    let service_id = service["id"].as_str().unwrap().to_string();
    let hours: Vec<Value> = (0..7)
        .map(|d| json!({"weekday": d, "start": "09:00", "end": "18:00"}))
        .collect();
    for (path, body) in [
        (
            format!("/t/shop-a/members/{staff_id}/working-hours"),
            json!({"hours": hours}),
        ),
        (
            format!("/t/shop-a/members/{staff_id}/services"),
            json!({"service_ids": [service_id]}),
        ),
    ] {
        let (s, b) = call(&app, Method::PUT, &path, Some(body), Some(&owner)).await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }
    Shop {
        app,
        owner,
        staff,
        staff_id,
        service_id,
    }
}

async fn delete_account(app: &Router, token: Option<&str>, password: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/auth/delete-account",
        Some(json!({ "password": password })),
        token,
    )
    .await
}

async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[sqlx::test]
async fn deleting_your_account_erases_who_you_are_and_frees_the_email(pool: PgPool) {
    let s = shop(&pool).await;
    // 各種會留著個人資料的東西
    sqlx::query(
        "INSERT INTO email_outbox (tenant_id, to_email, subject, body, status)
         SELECT NULL::uuid, 'b@example.com', '待寄', 'x', 'pending'
         UNION ALL SELECT NULL::uuid, 'b@example.com', '已寄', '', 'sent'",
    )
    .execute(&pool)
    .await
    .unwrap();
    call(
        &s.app,
        Method::POST,
        "/auth/forgot-password",
        Some(json!({"email": "b@example.com"})),
        None,
    )
    .await;
    sqlx::query(
        "INSERT INTO invitations (tenant_id, email, role, token_hash, expires_at, accepted_at)
         SELECT id, 'b@example.com', 'staff', 'h-accepted', now() + interval '1 day', now() FROM tenants",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO invitations (tenant_id, email, role, token_hash, expires_at)
         SELECT id, 'b@example.com', 'manager', 'h-pending', now() + interval '1 day' FROM tenants",
    )
    .execute(&pool)
    .await
    .unwrap();

    // 密碼不對:400,什麼都不動
    let (status, body) = delete_account(&s.app, Some(&s.staff), "wrong-password").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM users WHERE email = 'b@example.com'"
        )
        .await,
        1
    );
    // 沒登入:401
    assert_eq!(
        delete_account(&s.app, None, "password123").await.0,
        StatusCode::UNAUTHORIZED
    );

    let (status, body) = delete_account(&s.app, Some(&s.staff), "password123").await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // 帳號被匿名化,不是刪除:資料列還在(成員資格、稽核指著它)
    let (email, name, hash): (String, String, String) =
        sqlx::query_as("SELECT email::text, name, password_hash FROM users WHERE id = $1")
            .bind(s.staff_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(email, format!("deleted-{}@anonymized.invalid", s.staff_id));
    assert_eq!(name, "已刪除的使用者");
    assert!(!hash.starts_with("$argon2"), "密碼雜湊要被抹除");
    // 所有登入立刻失效、舊 Email 登入不了
    assert_eq!(
        call(&s.app, Method::GET, "/auth/me", None, Some(&s.staff))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &s.app,
            Method::POST,
            "/auth/login",
            Some(json!({"email": "b@example.com", "password": "password123"})),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // 成員資格停用、後台只看得到匿名的名字、稽核有紀錄
    assert_eq!(
        count(&pool, "SELECT count(*) FROM memberships WHERE active AND user_id IN (SELECT id FROM users WHERE name = '已刪除的使用者')").await,
        0
    );
    let (_, members) = call(
        &s.app,
        Method::GET,
        "/t/shop-a/members",
        None,
        Some(&s.owner),
    )
    .await;
    let text = members.to_string();
    assert!(
        text.contains("已刪除的使用者") && !text.contains("b@example.com"),
        "{text}"
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM audit_logs WHERE action = 'member.account_deleted'"
        )
        .await,
        1
    );
    // 寄給他的信:待寄的刪除、已寄的改匿名;重設密碼與邀請上的 Email 也一併處理
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM email_outbox WHERE lower(to_email) = 'b@example.com'"
        )
        .await,
        0
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM email_outbox WHERE status = 'pending' AND to_email LIKE '%anonymized.invalid'").await, 0);
    assert!(count(&pool, "SELECT count(*) FROM email_outbox WHERE status = 'sent' AND to_email LIKE '%anonymized.invalid'").await >= 1);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM password_resets").await,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM invitations WHERE email = 'b@example.com'"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM invitations WHERE email::text LIKE '%anonymized.invalid'"
        )
        .await,
        1,
        "已接受的邀請留著紀錄但不再指向真人;待處理的刪掉"
    );

    // 這個 Email 可以重新註冊(是一個全新的帳號)
    let (status, body) = register(&s.app, "b@example.com", "password123").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_ne!(body["user"]["id"].as_str().unwrap(), s.staff_id.to_string());
}

#[sqlx::test]
async fn a_deleted_account_cannot_be_reactivated_in_the_shop(pool: PgPool) {
    // 刪除帳號只是把成員資格停用;店家不能把這個「已刪除的使用者」重新啟用成可被預約的員工
    let s = shop(&pool).await;
    assert_eq!(
        delete_account(&s.app, Some(&s.staff), "password123")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    // 成員列表標出「已刪除」,前端才能不顯示重新啟用按鈕與匿名 Email
    let (_, members) = call(
        &s.app,
        Method::GET,
        "/t/shop-a/members",
        None,
        Some(&s.owner),
    )
    .await;
    let flags: Vec<(bool, bool)> = members
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["user_id"] == s.staff_id.to_string(),
                m["deleted"].as_bool().unwrap(),
            )
        })
        .collect();
    assert!(
        flags.iter().all(|(is_staff, deleted)| is_staff == deleted),
        "{members}"
    );
    let (status, body) = call(
        &s.app,
        Method::POST,
        &format!("/t/shop-a/members/{}/reactivate", s.staff_id),
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("刪除"), "{body}");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM memberships WHERE active AND role = 'staff'"
        )
        .await,
        0
    );
}

#[sqlx::test]
async fn an_owner_must_delete_their_shops_first(pool: PgPool) {
    let s = shop(&pool).await;
    let (status, body) = delete_account(&s.app, Some(&s.owner), "password123").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("擁有者"));
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM users WHERE email = 'a@example.com'"
        )
        .await,
        1
    );

    // 已經申請刪除店家(寬限期中)就可以了:店家會照期限刪掉
    let (status, body) = call(
        &s.app,
        Method::POST,
        "/t/shop-a/deletion",
        Some(json!({"confirm_slug": "shop-a"})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        delete_account(&s.app, Some(&s.owner), "password123")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test]
async fn staff_with_upcoming_bookings_cannot_delete_their_account(pool: PgPool) {
    let s = shop(&pool).await;
    let (status, booking) = call(
        &s.app,
        Method::POST,
        "/t/shop-a/bookings",
        Some(json!({
            "service_id": s.service_id,
            "staff_id": s.staff_id,
            "start": at(day(3), 10).to_rfc3339(),
            "customer": {"name": "王小明", "email": "c@example.com"},
        })),
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{booking}");

    let (status, body) = delete_account(&s.app, Some(&s.staff), "password123").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("預約"));
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM users WHERE email = 'b@example.com'"
        )
        .await,
        1
    );

    // 取消之後就可以
    let id = booking["id"].as_str().unwrap();
    let (status, _) = call(
        &s.app,
        Method::PATCH,
        &format!("/t/shop-a/bookings/{id}"),
        Some(json!({"status": "cancelled"})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        delete_account(&s.app, Some(&s.staff), "password123")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    // 歷史預約保留(只是負責人變成匿名)
    assert_eq!(count(&pool, "SELECT count(*) FROM bookings").await, 1);
}

#[sqlx::test]
async fn the_database_function_refuses_a_second_deletion(pool: PgPool) {
    let s = shop(&pool).await;
    assert_eq!(
        delete_account(&s.app, Some(&s.staff), "password123")
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let err = sqlx::query("SELECT delete_account($1)")
        .bind(s.staff_id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("P0003")
    );
}

#[sqlx::test]
async fn wrong_passwords_when_deleting_count_towards_the_account_lockout(pool: PgPool) {
    let s = shop(&pool).await;
    for _ in 0..5 {
        assert_eq!(
            delete_account(&s.app, Some(&s.staff), "wrong-password")
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    // 鎖定中:連正確的密碼都不收,帳號也沒被刪
    let (status, body) = delete_account(&s.app, Some(&s.staff), "password123").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("鎖定"), "{body}");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM users WHERE email = 'b@example.com'"
        )
        .await,
        1
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM email_outbox WHERE to_email = 'b@example.com' AND subject LIKE '%鎖定%'").await,
        1
    );
}
