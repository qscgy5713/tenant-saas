mod common;

use axum::{
    Router,
    http::{Method, StatusCode},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Asia::Taipei;
use common::{add_member, app, call, create_service, create_tenant, set_plan, signup, user_id};
use serde_json::{Value, json};
use sqlx::PgPool;
use tenant_saas::{
    availability::local_to_utc,
    db::{begin_scoped, begin_worker},
    mail::{Mailer, MemoryMailer},
    worker::tick,
};
use uuid::Uuid;

fn at(date: NaiveDate, h: u32, m: u32) -> DateTime<Utc> {
    local_to_utc(
        Taipei,
        date,
        chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap(),
    )
    .unwrap()
}

fn day(offset: i64) -> NaiveDate {
    Utc::now().with_timezone(&Taipei).date_naive() + Duration::days(offset)
}

struct Shop {
    app: Router,
    owner: String,
    owner_id: Uuid,
    service_id: String,
    tenant_id: Uuid,
}

/// 開店 shop-a,owner 每天 09:00–18:00 提供一個 60 分鐘的服務
async fn shop(pool: &PgPool) -> Shop {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    assert_eq!(
        create_tenant(&app, &owner, "shop-a").await.0,
        StatusCode::CREATED
    );
    let service_id = create_service(&app, &owner, "shop-a", "剪髮").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    call(
        &app,
        Method::PATCH,
        &format!("/t/shop-a/services/{service_id}"),
        Some(json!({"duration_minutes": 60})),
        Some(&owner),
    )
    .await;
    let owner_id = user_id(pool, "a@example.com").await;
    let hours: Vec<Value> = (0..7)
        .map(|d| json!({"weekday": d, "start": "09:00", "end": "18:00"}))
        .collect();
    call(
        &app,
        Method::PUT,
        &format!("/t/shop-a/members/{owner_id}/working-hours"),
        Some(json!({"hours": hours})),
        Some(&owner),
    )
    .await;
    call(
        &app,
        Method::PUT,
        &format!("/t/shop-a/members/{owner_id}/services"),
        Some(json!({"service_ids": [service_id]})),
        Some(&owner),
    )
    .await;
    let tenant_id = sqlx::query_scalar("SELECT id FROM tenants WHERE slug = 'shop-a'")
        .fetch_one(pool)
        .await
        .unwrap();
    Shop {
        app,
        owner,
        owner_id,
        service_id,
        tenant_id,
    }
}

async fn logs(app: &Router, token: &str, slug: &str, query: &str) -> Value {
    let (status, body) = call(
        app,
        Method::GET,
        &format!("/t/{slug}/audit-logs?{query}"),
        None,
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// 由舊到新的動作清單
async fn actions(app: &Router, token: &str, slug: &str) -> Vec<String> {
    let body = logs(app, token, slug, "limit=100").await;
    let mut list: Vec<String> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap().to_string())
        .collect();
    list.reverse();
    list
}

fn pg_code(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.to_string())
}

fn token_in(body: &str) -> String {
    body.split("/bookings/")
        .nth(1)
        .unwrap()
        .chars()
        .take(64)
        .collect()
}

// ---------- 內容與隱私 ----------

#[sqlx::test]
async fn management_actions_are_recorded_and_contain_no_personal_data(pool: PgPool) {
    let s = shop(&pool).await;
    let app = &s.app;
    let invitee = signup(app, "invitee@example.com").await;

    // 邀請 → 接受 → 升為 manager → 移除
    let (_, inv) = call(
        app,
        Method::POST,
        "/t/shop-a/invitations",
        Some(json!({"email": "invitee@example.com", "role": "staff"})),
        Some(&s.owner),
    )
    .await;
    let invite_token = inv["token"].as_str().unwrap().to_string();
    assert_eq!(
        call(
            app,
            Method::POST,
            "/invitations/accept",
            Some(json!({"token": invite_token})),
            Some(&invitee)
        )
        .await
        .0,
        StatusCode::OK
    );
    let invitee_id = user_id(&pool, "invitee@example.com").await;
    call(
        app,
        Method::PATCH,
        &format!("/t/shop-a/members/{invitee_id}"),
        Some(json!({"role": "manager"})),
        Some(&s.owner),
    )
    .await;
    call(
        app,
        Method::DELETE,
        &format!("/t/shop-a/members/{invitee_id}"),
        None,
        Some(&s.owner),
    )
    .await;

    // 休假(含私密原因)
    let off = json!({"starts_at": at(day(9), 9, 0).to_rfc3339(), "ends_at": at(day(9), 12, 0).to_rfc3339(), "reason": "SECRET_REASON 看診"});
    let (_, off) = call(
        app,
        Method::POST,
        &format!("/t/shop-a/members/{}/time-off", s.owner_id),
        Some(off),
        Some(&s.owner),
    )
    .await;
    call(
        app,
        Method::DELETE,
        &format!("/t/shop-a/time-off/{}", off["id"].as_str().unwrap()),
        None,
        Some(&s.owner),
    )
    .await;

    // 代客預約(含顧客 Email、備註)
    let (status, booking) = call(app, Method::POST, "/t/shop-a/bookings", Some(json!({
        "service_id": s.service_id, "start": at(day(3), 10, 0).to_rfc3339(),
        "customer": {"name": "秘密顧客", "email": "secret-customer@example.com", "phone": "0912-345-678"},
    })), Some(&s.owner)).await;
    assert_eq!(status, StatusCode::CREATED);
    let manage_token = booking["manage_token"].as_str().unwrap().to_string();
    let booking_id = booking["id"].as_str().unwrap();
    call(
        app,
        Method::PATCH,
        &format!("/t/shop-a/bookings/{booking_id}"),
        Some(json!({"notes": "SECRET_NOTE 過敏"})),
        Some(&s.owner),
    )
    .await;

    // 第二個服務:建立 → 刪除
    let svc2 = create_service(app, &s.owner, "shop-a", "染髮").await;
    call(
        app,
        Method::DELETE,
        &format!("/t/shop-a/services/{}", svc2["id"].as_str().unwrap()),
        None,
        Some(&s.owner),
    )
    .await;

    assert_eq!(
        actions(app, &s.owner, "shop-a").await,
        vec![
            "tenant.created",
            "service.created",
            "service.updated",
            "member.working_hours_replaced",
            "member.services_replaced",
            "invitation.created",
            "invitation.accepted",
            "member.role_changed",
            "member.removed",
            "time_off.created",
            "time_off.deleted",
            "booking.created",
            "booking.notes_updated",
            "service.created",
            "service.deleted",
        ]
    );

    // 稽核紀錄只能新增、刪不掉:裡面絕不能有個資、token、私密原因
    let everything: String = sqlx::query_scalar("SELECT coalesce(string_agg(detail::text || action || entity_type, ' '), '') FROM audit_logs")
        .fetch_one(&pool).await.unwrap();
    for secret in [
        "invitee@example.com",
        "secret-customer",
        "0912",
        "SECRET_REASON",
        "SECRET_NOTE",
        &invite_token,
        &manage_token,
    ] {
        assert!(!everything.contains(secret), "稽核紀錄不該含 {secret}");
    }
    assert!(!everything.contains('@'), "稽核紀錄不該含任何 Email");

    // 內容:誰、對什麼、轉換
    let all = logs(app, &s.owner, "shop-a", "limit=100").await;
    let items = all["items"].as_array().unwrap();
    let role = items
        .iter()
        .find(|e| e["action"] == "member.role_changed")
        .unwrap();
    assert_eq!(role["detail"], json!({"from": "staff", "to": "manager"}));
    assert_eq!(role["entity_id"], invitee_id.to_string());
    assert_eq!(role["actor_type"], "user");
    assert_eq!(role["actor_user_id"], s.owner_id.to_string());
    assert_eq!(role["actor_name"], "小明");
    let created = items
        .iter()
        .find(|e| e["action"] == "booking.created")
        .unwrap();
    assert_eq!(created["detail"]["source"], "staff");
    let notes = items
        .iter()
        .find(|e| e["action"] == "booking.notes_updated")
        .unwrap();
    assert_eq!(notes["detail"], json!({"notes_changed": true}));
}

#[sqlx::test]
async fn failed_operations_leave_no_audit_rows(pool: PgPool) {
    let s = shop(&pool).await;
    // 免費版 5 項服務:setup 已有 1 項,再建 4 項,第 6 項被擋
    for i in 0..4 {
        create_service(&s.app, &s.owner, "shop-a", &format!("服務{i}")).await;
    }
    let (status, _) = call(
        &s.app,
        Method::POST,
        "/t/shop-a/services",
        Some(json!({"name": "第六項", "duration_minutes": 30})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    // 重複邀請:第二次 409(升到有名額的方案,否則會先撞到名額上限)
    set_plan(&pool, "shop-a", "pro").await;
    let invite = || {
        call(
            &s.app,
            Method::POST,
            "/t/shop-a/invitations",
            Some(json!({"email": "x@example.com", "role": "staff"})),
            Some(&s.owner),
        )
    };
    assert_eq!(invite().await.0, StatusCode::CREATED);
    assert_eq!(invite().await.0, StatusCode::CONFLICT);
    // 無權限、找不到
    let staff = signup(&s.app, "s@example.com").await;
    add_member(&pool, "shop-a", "s@example.com", "staff").await;
    call(
        &s.app,
        Method::POST,
        "/t/shop-a/services",
        Some(json!({"name": "x", "duration_minutes": 30})),
        Some(&staff),
    )
    .await;
    call(
        &s.app,
        Method::DELETE,
        &format!("/t/shop-a/services/{}", Uuid::new_v4()),
        None,
        Some(&s.owner),
    )
    .await;
    call(
        &s.app,
        Method::PATCH,
        &format!("/t/shop-a/services/{}", Uuid::new_v4()),
        Some(json!({"name": "x"})),
        Some(&s.owner),
    )
    .await;

    let list = actions(&s.app, &s.owner, "shop-a").await;
    assert_eq!(
        list.iter().filter(|a| *a == "service.created").count(),
        5,
        "被擋下的第 6 項不留紀錄"
    );
    assert_eq!(
        list.iter().filter(|a| *a == "invitation.created").count(),
        1
    );
    assert_eq!(list.iter().filter(|a| *a == "service.deleted").count(), 0);
    assert_eq!(
        list.iter().filter(|a| *a == "service.updated").count(),
        1,
        "只有 setup 那一次,找不到的 PATCH 不留紀錄"
    );
}

// ---------- 存取與隔離 ----------

#[sqlx::test]
async fn only_managers_can_read_and_each_shop_sees_only_its_own(pool: PgPool) {
    let s = shop(&pool).await;
    let staff = signup(&s.app, "s@example.com").await;
    let manager = signup(&s.app, "m@example.com").await;
    let stranger = signup(&s.app, "z@example.com").await;
    add_member(&pool, "shop-a", "s@example.com", "staff").await;
    add_member(&pool, "shop-a", "m@example.com", "manager").await;

    let uri = "/t/shop-a/audit-logs";
    assert_eq!(
        call(&s.app, Method::GET, uri, None, Some(&staff)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&s.app, Method::GET, uri, None, Some(&manager)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&s.app, Method::GET, uri, None, Some(&stranger))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&s.app, Method::GET, uri, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );

    // 同一個人擁有兩家店:各看各的
    create_tenant(&s.app, &s.owner, "shop-b").await;
    create_service(&s.app, &s.owner, "shop-b", "B 的服務").await;
    let a = logs(&s.app, &s.owner, "shop-a", "limit=100").await;
    let b = logs(&s.app, &s.owner, "shop-b", "limit=100").await;
    assert_eq!(
        b["items"].as_array().unwrap().len(),
        2,
        "tenant.created + service.created"
    );
    let a_ids: Vec<i64> = a["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_i64().unwrap())
        .collect();
    for e in b["items"].as_array().unwrap() {
        assert!(!a_ids.contains(&e["id"].as_i64().unwrap()));
    }
    // 沒有任何寫入端點
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        let (status, _) = call(&s.app, method, uri, Some(json!({})), Some(&s.owner)).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }
}

#[sqlx::test]
async fn audit_log_is_append_only_at_the_database_level(pool: PgPool) {
    let s = shop(&pool).await;
    let scoped = || begin_scoped(&pool, Some(s.owner_id), Some(s.tenant_id));
    let other_tenant: Uuid = {
        let u = signup(&s.app, "z@example.com").await;
        create_tenant(&s.app, &u, "shop-z").await;
        sqlx::query_scalar("SELECT id FROM tenants WHERE slug = 'shop-z'")
            .fetch_one(&pool)
            .await
            .unwrap()
    };

    // 請求內的角色:能讀、能新增,但改不了、刪不了,也不能替別家店寫
    let mut tx = scoped().await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(n >= 5);
    drop(tx);
    for sql in [
        "UPDATE audit_logs SET action = 'tampered'",
        "UPDATE audit_logs SET detail = '{}'",
        "DELETE FROM audit_logs",
        "TRUNCATE audit_logs",
    ] {
        let mut tx = scoped().await.unwrap();
        let err = sqlx::query(sql).execute(&mut *tx).await.unwrap_err();
        assert_eq!(pg_code(&err).as_deref(), Some("42501"), "{sql}");
    }
    let mut tx = scoped().await.unwrap();
    let err = sqlx::query("INSERT INTO audit_logs (tenant_id, actor_type, action, entity_type) VALUES ($1, 'system', 'forged', 'x')")
        .bind(other_tenant).execute(&mut *tx).await.unwrap_err();
    assert_eq!(pg_code(&err).as_deref(), Some("42501"), "不能替別家店寫入");

    // worker:只能新增,連讀都不行
    for sql in [
        "UPDATE audit_logs SET action = 'x'",
        "DELETE FROM audit_logs",
        "SELECT * FROM audit_logs",
    ] {
        let mut tx = begin_worker(&pool).await.unwrap();
        let err = sqlx::query(sql).execute(&mut *tx).await.unwrap_err();
        assert_eq!(pg_code(&err).as_deref(), Some("42501"), "worker: {sql}");
    }

    // 資料一致性:使用者行為必須有 actor_user_id,系統 / 顧客不能有
    for sql in [
        "INSERT INTO audit_logs (tenant_id, actor_type, action, entity_type) VALUES (gen_random_uuid(), 'user', 'a', 'b')",
        "INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type) VALUES ($1, 'system', gen_random_uuid(), 'a', 'b')",
    ] {
        let err = sqlx::query(sql)
            .bind(s.tenant_id)
            .execute(&pool)
            .await
            .unwrap_err();
        assert!(
            matches!(pg_code(&err).as_deref(), Some("23514") | Some("23503")),
            "{sql}: {err}"
        );
    }
}

// ---------- 顧客與系統的行為 ----------

fn mailer() -> (Mailer, MemoryMailer) {
    let m = MemoryMailer::new();
    (Mailer::Memory(m.clone()), m)
}

async fn request(s: &Shop, start: DateTime<Utc>, email: &str) -> Value {
    let (status, body) = call(&s.app, Method::POST, "/public/shops/shop-a/bookings", Some(json!({
        "service_id": s.service_id, "start": start.to_rfc3339(), "customer": {"name": "顧客", "email": email},
    })), None).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

#[sqlx::test]
async fn customer_actions_are_audited_with_customer_as_actor(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let start = at(day(3), 10, 0);

    let req = request(&s, start, "c@example.com").await;
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    let token = token_in(&memory.sent()[0].body);
    let id = req["id"].as_str().unwrap();

    call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/confirm"),
        None,
        None,
    )
    .await;
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/reschedule"),
        Some(json!({"start": (start + Duration::hours(2)).to_rfc3339()})),
        None,
    )
    .await;
    call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{token}/cancel"),
        None,
        None,
    )
    .await;

    let body = logs(
        &s.app,
        &s.owner,
        "shop-a",
        &format!("entity_id={id}&limit=100"),
    )
    .await;
    let items = body["items"].as_array().unwrap();
    let acts: Vec<&str> = items
        .iter()
        .rev()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        acts,
        vec![
            "booking.requested",
            "booking.confirmed",
            "booking.rescheduled",
            "booking.cancelled"
        ]
    );
    for e in items {
        assert_eq!(e["actor_type"], "customer");
        assert!(e["actor_user_id"].is_null());
    }
    let resched = items
        .iter()
        .find(|e| e["action"] == "booking.rescheduled")
        .unwrap();
    assert_eq!(
        resched["detail"]["from_starts_at"]
            .as_str()
            .unwrap()
            .parse::<DateTime<Utc>>()
            .unwrap(),
        start
    );
    assert_eq!(
        resched["detail"]["to_starts_at"]
            .as_str()
            .unwrap()
            .parse::<DateTime<Utc>>()
            .unwrap(),
        start + Duration::hours(2)
    );
    let cancelled = items
        .iter()
        .find(|e| e["action"] == "booking.cancelled")
        .unwrap();
    assert_eq!(
        cancelled["detail"],
        json!({"from": "confirmed", "to": "cancelled"})
    );
}

#[sqlx::test]
async fn system_cancellations_record_the_reason(pool: PgPool) {
    let s = shop(&pool).await;
    let (mailer, memory) = mailer();
    let start = at(day(3), 10, 0);

    // 兩人申請同一時段,後確認的那位被系統取消
    request(&s, start, "first@example.com").await;
    request(&s, start, "second@example.com").await;
    // 第三位:申請後超過有效期,由 worker 整理(confirmation_expired)
    request(&s, start + Duration::hours(4), "stale@example.com").await;
    tick(&pool, &mailer, "http://app.test").await.unwrap();
    let token_of = |email: &str| {
        token_in(
            &memory
                .sent()
                .into_iter()
                .find(|m| m.to == email)
                .unwrap()
                .body,
        )
    };
    call(
        &s.app,
        Method::POST,
        &format!("/public/bookings/{}/confirm", token_of("first@example.com")),
        None,
        None,
    )
    .await;
    let (status, _) = call(
        &s.app,
        Method::POST,
        &format!(
            "/public/bookings/{}/confirm",
            token_of("second@example.com")
        ),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    sqlx::query("UPDATE bookings SET created_at = now() - interval '25 hours' WHERE customer_id IN (SELECT id FROM customers WHERE email = 'stale@example.com')")
        .execute(&pool).await.unwrap();
    tick(&pool, &mailer, "http://app.test").await.unwrap();

    let body = logs(
        &s.app,
        &s.owner,
        "shop-a",
        "action=booking.cancelled&limit=100",
    )
    .await;
    let mut reasons: Vec<String> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            assert_eq!(e["actor_type"], "system");
            e["detail"]["reason"].as_str().unwrap().to_string()
        })
        .collect();
    reasons.sort();
    // 依序確認時,先確認的已占位,後者在「確認前重新檢查」就被發現不能用(no_longer_bookable);
    // slot_taken 只會在兩個確認真正同時發生、由排除約束擋下時出現
    assert_eq!(reasons, vec!["confirmation_expired", "no_longer_bookable"]);
}

#[sqlx::test]
async fn staff_can_cancel_a_pending_booking_but_not_complete_it(pool: PgPool) {
    let s = shop(&pool).await;
    let req = request(&s, at(day(3), 10, 0), "c@example.com").await;
    let id = req["id"].as_str().unwrap();
    let patch = |status: &str| {
        let (app, owner, uri, status) = (
            s.app.clone(),
            s.owner.clone(),
            format!("/t/shop-a/bookings/{id}"),
            status.to_string(),
        );
        async move {
            call(
                &app,
                Method::PATCH,
                &uri,
                Some(json!({"status": status})),
                Some(&owner),
            )
            .await
        }
    };

    assert_eq!(
        patch("completed").await.0,
        StatusCode::BAD_REQUEST,
        "尚未確認不能標記完成"
    );
    assert_eq!(patch("no_show").await.0, StatusCode::BAD_REQUEST);
    let (status, body) = patch("cancelled").await;
    assert_eq!(status, StatusCode::OK, "待確認的預約員工應能取消: {body}");
    assert_eq!(body["status"], "cancelled");
    assert_eq!(
        patch("cancelled").await.0,
        StatusCode::CONFLICT,
        "取消是終態"
    );

    let items = logs(
        &s.app,
        &s.owner,
        "shop-a",
        &format!("entity_id={id}&action=booking.cancelled"),
    )
    .await;
    let entry = &items["items"][0];
    assert_eq!(entry["actor_type"], "user");
    assert_eq!(
        entry["detail"],
        json!({"from": "pending", "to": "cancelled", "notes_changed": false})
    );
}

// ---------- 查詢 ----------

#[sqlx::test]
async fn filters_and_cursor_pagination(pool: PgPool) {
    let s = shop(&pool).await;
    set_plan(&pool, "shop-a", "business").await;
    for i in 0..7 {
        create_service(&s.app, &s.owner, "shop-a", &format!("服務{i}")).await;
    }
    let total = logs(&s.app, &s.owner, "shop-a", "limit=100").await["items"]
        .as_array()
        .unwrap()
        .len();

    // 用游標翻頁:每頁 4 筆,id 嚴格遞減、不重複、不遺漏
    let mut seen: Vec<i64> = Vec::new();
    let mut before: Option<i64> = None;
    let mut pages = 0;
    loop {
        let q = match before {
            Some(b) => format!("limit=4&before={b}"),
            None => "limit=4".to_string(),
        };
        let page = logs(&s.app, &s.owner, "shop-a", &q).await;
        seen.extend(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["id"].as_i64().unwrap()),
        );
        pages += 1;
        match page["next_before"].as_i64() {
            Some(b) => before = Some(b),
            None => break,
        }
        assert!(pages < 20);
    }
    assert_eq!(seen.len(), total);
    assert!(seen.windows(2).all(|w| w[0] > w[1]));

    // limit 夾在 1..100
    assert_eq!(
        logs(&s.app, &s.owner, "shop-a", "limit=0").await["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        logs(&s.app, &s.owner, "shop-a", "limit=100000").await["items"]
            .as_array()
            .unwrap()
            .len()
            <= 100
    );

    // 篩選:前綴、實體、操作者、時間
    let n = |q: &str| {
        let (app, owner, q) = (s.app.clone(), s.owner.clone(), q.to_string());
        async move {
            logs(&app, &owner, "shop-a", &q).await["items"]
                .as_array()
                .unwrap()
                .len()
        }
    };
    assert_eq!(n("action=service.created").await, 8);
    assert_eq!(
        n("action=service.").await,
        9,
        "前綴:8 次 created + 1 次 updated"
    );
    assert_eq!(n("action=member.").await, 2);
    assert_eq!(n("entity_type=service").await, 9);
    assert_eq!(n(&format!("actor_user_id={}", s.owner_id)).await, total);
    assert_eq!(n(&format!("actor_user_id={}", Uuid::new_v4())).await, 0);
    assert_eq!(n("from=2999-01-01T00:00:00Z").await, 0);
    assert_eq!(n("to=2000-01-01T00:00:00Z").await, 0);
    // LIKE 萬用字元不能被當成萬用字元
    assert_eq!(n("action=%25").await, 0);
    assert_eq!(n("action=_ervice.created").await, 0);
    assert_eq!(
        call(
            &s.app,
            Method::GET,
            &format!("/t/shop-a/audit-logs?action={}", "x".repeat(65)),
            None,
            Some(&s.owner)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

/// 前面的測試沒走到的動作:撤銷邀請、自己退出、標記完成、標記未到
#[sqlx::test]
async fn remaining_actions_are_recorded(pool: PgPool) {
    let s = shop(&pool).await;
    set_plan(&pool, "shop-a", "pro").await;

    // 撤銷邀請
    let (_, inv) = call(
        &s.app,
        Method::POST,
        "/t/shop-a/invitations",
        Some(json!({"email": "x@example.com", "role": "staff"})),
        Some(&s.owner),
    )
    .await;
    let inv_id = inv["id"].as_str().unwrap();
    assert_eq!(
        call(
            &s.app,
            Method::DELETE,
            &format!("/t/shop-a/invitations/{inv_id}"),
            None,
            Some(&s.owner)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );

    // 員工自己退出
    let staff = signup(&s.app, "s@example.com").await;
    add_member(&pool, "shop-a", "s@example.com", "staff").await;
    let staff_id = user_id(&pool, "s@example.com").await;
    assert_eq!(
        call(
            &s.app,
            Method::DELETE,
            &format!("/t/shop-a/members/{staff_id}"),
            None,
            Some(&staff)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );

    // 完成與未到:預約的開始時間要已經過了
    let mut ids = Vec::new();
    for (h, email) in [(10, "c1@example.com"), (12, "c2@example.com")] {
        let (_, b) = call(&s.app, Method::POST, "/t/shop-a/bookings", Some(json!({
            "service_id": s.service_id, "start": at(day(3), h, 0).to_rfc3339(), "customer": {"name": "顧客", "email": email},
        })), Some(&s.owner)).await;
        ids.push(b["id"].as_str().unwrap().to_string());
    }
    sqlx::query("UPDATE bookings SET starts_at = now() - interval '3 hours', ends_at = now() - interval '2 hours' WHERE id = $1::uuid").bind(&ids[0]).execute(&pool).await.unwrap();
    sqlx::query("UPDATE bookings SET starts_at = now() - interval '2 hours', ends_at = now() - interval '1 hour' WHERE id = $1::uuid").bind(&ids[1]).execute(&pool).await.unwrap();
    for (id, status) in [(&ids[0], "completed"), (&ids[1], "no_show")] {
        let (code, _) = call(
            &s.app,
            Method::PATCH,
            &format!("/t/shop-a/bookings/{id}"),
            Some(json!({"status": status})),
            Some(&s.owner),
        )
        .await;
        assert_eq!(code, StatusCode::OK, "{status}");
    }

    let list = actions(&s.app, &s.owner, "shop-a").await;
    for expected in [
        "invitation.revoked",
        "member.left",
        "booking.completed",
        "booking.no_show",
    ] {
        assert_eq!(
            list.iter().filter(|a| *a == expected).count(),
            1,
            "{expected}"
        );
    }
    let left = logs(&s.app, &s.owner, "shop-a", "action=member.left").await;
    assert_eq!(left["items"][0]["actor_user_id"], staff_id.to_string());
    assert_eq!(left["items"][0]["entity_id"], staff_id.to_string());
    let done = logs(&s.app, &s.owner, "shop-a", "action=booking.completed").await;
    assert_eq!(
        done["items"][0]["detail"],
        json!({"from": "confirmed", "to": "completed", "notes_changed": false})
    );
}
