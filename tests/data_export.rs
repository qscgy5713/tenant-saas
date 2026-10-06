//! 匯出預約(CSV)與刪除顧客個資(匿名化)。

mod common;

use axum::{Router, http::Method, http::StatusCode};
use chrono::{Duration, Utc};
use common::{
    add_member, app, call, create_service, create_tenant, parse_csv, raw_get, signup, user_id,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

struct Shop {
    app: Router,
    owner: String,
    owner_id: Uuid,
    service: String,
    tenant_id: Uuid,
}

async fn shop(pool: &PgPool) -> Shop {
    let app = app(pool.clone());
    let owner = signup(&app, "o@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    let svc = create_service(&app, &owner, "shop-a", "剪髮").await;
    let owner_id = user_id(pool, "o@example.com").await;
    call(
        &app,
        Method::PUT,
        &format!("/t/shop-a/members/{owner_id}/services"),
        Some(json!({"service_ids": [svc["id"]]})),
        Some(&owner),
    )
    .await;
    let hours: Vec<Value> = (0..7)
        .map(|d| json!({"weekday": d, "start": "00:00", "end": "23:59"}))
        .collect();
    call(
        &app,
        Method::PUT,
        &format!("/t/shop-a/members/{owner_id}/working-hours"),
        Some(json!({"hours": hours})),
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
        service: svc["id"].as_str().unwrap().to_string(),
        tenant_id,
    }
}

/// 店主代客預約(直接成立)。回傳預約 id
async fn book(s: &Shop, days: i64, hour: u32, name: &str, email: &str, phone: &str) -> String {
    let start = (Utc::now() + Duration::days(days))
        .format(&format!("%Y-%m-%dT{hour:02}:00:00Z"))
        .to_string();
    let (status, body) = call(
        &s.app,
        Method::POST,
        "/t/shop-a/bookings",
        Some(json!({"service_id": s.service, "start": start,
            "customer": {"name": name, "email": email, "phone": phone}})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_string()
}

async fn set_notes(s: &Shop, id: &str, notes: &str) {
    let (status, _) = call(
        &s.app,
        Method::PATCH,
        &format!("/t/shop-a/bookings/{id}"),
        Some(json!({"notes": notes})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

/// 已經結束的預約(API 不允許建立過去的預約,直接寫入)
async fn past_booking(pool: &PgPool, s: &Shop, customer: Uuid, notes: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO bookings (tenant_id, staff_user_id, service_id, customer_id, starts_at, ends_at,
                               status, notes, manage_token_hash, confirmed_at)
         VALUES ($1, $2, $3, $4, now() - interval '3 days', now() - interval '3 days' + interval '1 hour',
                 'completed', $5, md5(random()::text) || md5(random()::text), now() - interval '4 days')
         RETURNING id",
    )
    .bind(s.tenant_id)
    .bind(s.owner_id)
    .bind(s.service.parse::<Uuid>().unwrap())
    .bind(customer)
    .bind(notes)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn customer_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM customers WHERE email = $1::citext")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap()
}

const COL_STATUS: usize = 1;
const COL_START_UTC: usize = 2;
const COL_START_LOCAL: usize = 3;
const COL_CUSTOMER: usize = 8;
const COL_EMAIL: usize = 9;
const COL_PHONE: usize = 10;
const COL_NOTES: usize = 11;

async fn export(
    s: &Shop,
    token: Option<&str>,
    query: &str,
) -> (StatusCode, Vec<Vec<String>>, String) {
    let (status, _, body) = raw_get(
        &s.app,
        token,
        &format!("/t/shop-a/bookings/export.csv{query}"),
    )
    .await;
    let rows = if status == StatusCode::OK {
        parse_csv(&body)
    } else {
        Vec::new()
    };
    (status, rows, body)
}

async fn exported_audits(pool: &PgPool) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT detail FROM audit_logs WHERE action = 'booking.exported' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

// ---------- 匯出預約 ----------

#[sqlx::test]
async fn bookings_export_is_a_safe_csv_with_local_times(pool: PgPool) {
    let s = shop(&pool).await;
    // 顧客姓名是外人填的:公式、逗號、引號、換行都要安全
    let evil_name = "=HYPERLINK(\"http://evil.example\",\"x\"),王";
    let id = book(&s, 3, 2, evil_name, "ming@example.com", "0912-345-678").await;
    set_notes(&s, &id, "VIP,\n過敏:\"染劑\"").await;
    book(&s, 2, 5, "李小華", "hua@example.com", "").await;

    let (status, rows, body) = export(&s, Some(&s.owner), "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(rows.iter().all(|r| r.len() == 12), "每列 12 欄");
    assert_eq!(rows.len(), 3);
    // 由早到晚
    assert_eq!(rows[1][COL_CUSTOMER], "李小華");
    let r = &rows[2];
    assert_eq!(r[COL_STATUS], "已確認");
    assert_eq!(r[COL_CUSTOMER], format!("'{evil_name}"), "公式被加上單引號");
    assert_eq!(r[COL_EMAIL], "ming@example.com");
    assert_eq!(r[COL_PHONE], "0912-345-678");
    assert_eq!(
        r[COL_NOTES], "VIP,\n過敏:\"染劑\"",
        "含逗號 / 換行 / 引號的備註來回無損"
    );
    assert!(
        !rows
            .iter()
            .flatten()
            .any(|c| c.starts_with('=') || c.starts_with('+') || c.starts_with('@')),
        "沒有任何欄位以公式字元開頭"
    );
    // 當地時間 = UTC + 8(台北);UTC 欄以 Z 結尾
    let utc: chrono::DateTime<Utc> = r[COL_START_UTC].parse().unwrap();
    assert!(r[COL_START_UTC].ends_with('Z'));
    assert_eq!(
        r[COL_START_LOCAL],
        utc.with_timezone(&chrono_tz::Asia::Taipei)
            .format("%Y-%m-%d %H:%M")
            .to_string()
    );

    // 匯出留下一筆稽核:誰、條件、幾筆 —— 但沒有任何顧客個資
    let audits = exported_audits(&pool).await;
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0]["rows"], 2);
    let all: String = sqlx::query_scalar("SELECT string_agg(detail::text, ' ') FROM audit_logs")
        .fetch_one(&pool)
        .await
        .unwrap();
    for pii in [
        "王",
        "ming@example.com",
        "0912-345-678",
        "VIP",
        "hua@example.com",
    ] {
        assert!(!all.contains(pii), "稽核不該有個資 {pii}");
    }
}

#[sqlx::test]
async fn export_filters_match_the_list_semantics(pool: PgPool) {
    let s = shop(&pool).await;
    let a = book(&s, 2, 3, "甲", "a@example.com", "").await;
    book(&s, 5, 3, "乙", "b@example.com", "").await;
    book(&s, 9, 3, "丙", "c@example.com", "").await;
    // 取消甲
    call(
        &s.app,
        Method::PATCH,
        &format!("/t/shop-a/bookings/{a}"),
        Some(json!({"status": "cancelled"})),
        Some(&s.owner),
    )
    .await;

    let names = |rows: &[Vec<String>]| -> Vec<String> {
        rows[1..].iter().map(|r| r[COL_CUSTOMER].clone()).collect()
    };
    let (_, all, _) = export(&s, Some(&s.owner), "").await;
    assert_eq!(
        names(&all),
        ["甲", "乙", "丙"],
        "不篩選就是全部,包含已取消的"
    );

    let (_, cancelled, _) = export(&s, Some(&s.owner), "?status=cancelled").await;
    assert_eq!(names(&cancelled), ["甲"]);
    assert_eq!(cancelled[1][COL_STATUS], "已取消");

    let from = (Utc::now() + Duration::days(4)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let to = (Utc::now() + Duration::days(7)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let (_, ranged, _) = export(&s, Some(&s.owner), &format!("?from={from}&to={to}")).await;
    assert_eq!(names(&ranged), ["乙"]);

    let (_, mine, _) = export(&s, Some(&s.owner), &format!("?staff_id={}", s.owner_id)).await;
    assert_eq!(names(&mine).len(), 3);
    let (_, none, _) = export(&s, Some(&s.owner), &format!("?staff_id={}", Uuid::new_v4())).await;
    assert!(names(&none).is_empty());
    let audits = exported_audits(&pool).await;
    assert_eq!(audits.len(), 5);
    assert_eq!(audits[1]["status"], "cancelled");
}

#[sqlx::test]
async fn export_is_for_managers_and_stays_inside_the_shop(pool: PgPool) {
    let s = shop(&pool).await;
    book(&s, 2, 3, "甲", "a@example.com", "").await;
    let staff = signup(&s.app, "staff@example.com").await;
    add_member(&pool, "shop-a", "staff@example.com", "staff").await;
    let outsider = signup(&s.app, "x@example.com").await;
    let manager = signup(&s.app, "m@example.com").await;
    add_member(&pool, "shop-a", "m@example.com", "manager").await;

    assert_eq!(export(&s, Some(&staff), "").await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        export(&s, Some(&outsider), "").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(export(&s, None, "").await.0, StatusCode::UNAUTHORIZED);
    assert!(exported_audits(&pool).await.is_empty(), "被拒絕的不留紀錄");
    assert_eq!(export(&s, Some(&manager), "").await.0, StatusCode::OK);

    // 另一家店的匯出看不到這家店的顧客
    let other = signup(&s.app, "b@example.com").await;
    create_tenant(&s.app, &other, "shop-b").await;
    let (status, _, body) = raw_get(&s.app, Some(&other), "/t/shop-b/bookings/export.csv").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse_csv(&body).len(), 1, "只有標題列");
}

/// 邊界:剛好 50,000 筆可以;多一筆被拒絕,而且被拒絕的匯出不留紀錄
#[sqlx::test]
async fn bookings_export_row_limit_is_exact(pool: PgPool) {
    const MAX: i64 = 50_000; // 對外承諾的上限,刻意寫死
    let s = shop(&pool).await;
    let id = book(&s, 2, 3, "甲", "a@example.com", "").await;
    let customer = customer_id(&pool, "a@example.com").await;
    let _ = id;
    let fill = |n: i64| {
        let pool = pool.clone();
        let (tenant, owner, service) =
            (s.tenant_id, s.owner_id, s.service.parse::<Uuid>().unwrap());
        async move {
            // 已取消的預約不受「同一時間只能有一筆」的排除限制,可以大量寫入
            sqlx::query(
                "INSERT INTO bookings (tenant_id, staff_user_id, service_id, customer_id, starts_at, ends_at,
                                       status, manage_token_hash)
                 SELECT $1, $2, $3, $4, now() + interval '30 days', now() + interval '30 days 1 hour',
                        'cancelled', md5(random()::text || g::text) || md5(g::text)
                 FROM generate_series(1, $5::int) g",
            )
            .bind(tenant).bind(owner).bind(service).bind(customer).bind(n)
            .execute(&pool).await.unwrap();
        }
    };
    fill(MAX - 1).await; // 加上開頭那一筆 = 剛好 50,000
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM bookings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, MAX);

    let (status, rows, _) = export(&s, Some(&s.owner), "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows.len() as i64, MAX + 1);
    assert_eq!(exported_audits(&pool).await.len(), 1);

    fill(1).await;
    let (status, _, body) = export(&s, Some(&s.owner), "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("50000") && body.contains("縮小"), "{body}");
    assert_eq!(
        exported_audits(&pool).await.len(),
        1,
        "被拒絕的匯出沒帶走資料,不留紀錄"
    );

    // 縮小範圍就可以
    let (status, rows, _) = export(&s, Some(&s.owner), "?status=confirmed").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows.len(), 2);
}

// ---------- 刪除顧客個資(匿名化) ----------

async fn anonymize(s: &Shop, token: &str, id: Uuid) -> (StatusCode, Value) {
    call(
        &s.app,
        Method::POST,
        &format!("/t/shop-a/customers/{id}/anonymize"),
        None,
        Some(token),
    )
    .await
}

#[sqlx::test]
async fn anonymizing_erases_who_the_customer_is_but_keeps_the_bookings(pool: PgPool) {
    let s = shop(&pool).await;
    let customer = {
        let id = book(&s, 40, 3, "王小明", "ming@example.com", "0912-345-678").await;
        let c = customer_id(&pool, "ming@example.com").await;
        // 這筆是未來的:先取消,才能刪
        call(
            &s.app,
            Method::PATCH,
            &format!("/t/shop-a/bookings/{id}"),
            Some(json!({"status": "cancelled", "notes": "對花粉過敏,電話 0912-345-678"})),
            Some(&s.owner),
        )
        .await;
        c
    };
    let past = past_booking(&pool, &s, customer, "常客,愛喝茶").await;
    // 信件佇列:一封還沒寄、一封已寄出
    for (status, subject) in [("pending", "待寄"), ("sent", "已寄")] {
        sqlx::query("INSERT INTO email_outbox (tenant_id, to_email, subject, body, status) VALUES ($1, 'Ming@Example.com', $2, 'hi', $3)")
            .bind(s.tenant_id).bind(subject).bind(status)
            .execute(&pool).await.unwrap();
    }
    // 別的顧客不受影響
    book(&s, 41, 5, "李小華", "hua@example.com", "0988-111-222").await;

    let (status, body) = anonymize(&s, &s.owner, customer).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // 顧客資料被抹掉
    let (name, email, phone): (String, String, Option<String>) =
        sqlx::query_as("SELECT name, email::text, phone FROM customers WHERE id = $1")
            .bind(customer)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(name, "已刪除的顧客");
    assert_eq!(email, format!("deleted-{customer}@anonymized.invalid"));
    assert_eq!(phone, None);
    // 預約保留(統計用),備註清掉
    let kept: (i64, i64) =
        sqlx::query_as("SELECT count(*), count(notes) FROM bookings WHERE customer_id = $1")
            .bind(customer)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(kept, (2, 0));
    let status_of_past: String =
        sqlx::query_scalar("SELECT status::text FROM bookings WHERE id = $1")
            .bind(past)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status_of_past, "completed");
    // 沒寄出的信刪掉;已寄出的紀錄不再指向真人
    let mails: Vec<(String, String)> = sqlx::query_as(
        "SELECT to_email, status FROM email_outbox WHERE to_email <> 'hua@example.com' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(mails, [(email.clone(), "sent".to_string())]);
    // 另一位顧客(李小華)那封確認信不受影響
    let hua_mail: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM email_outbox WHERE to_email = 'hua@example.com' AND status = 'pending'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(hua_mail, 1);

    // 整個資料庫裡再也找不到這個人(所有可能存放個資的地方)
    let leaks: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM customers WHERE name LIKE '%王小明%' OR email::text ILIKE '%ming@%' OR phone LIKE '%0912%')
              + (SELECT count(*) FROM bookings WHERE notes LIKE '%0912%' OR notes LIKE '%花粉%' OR notes LIKE '%愛喝茶%')
              + (SELECT count(*) FROM email_outbox WHERE lower(to_email) LIKE '%ming@%' OR subject LIKE '%王小明%')
              + (SELECT count(*) FROM audit_logs WHERE detail::text LIKE '%王小明%' OR detail::text LIKE '%ming@%' OR detail::text LIKE '%0912%')",
    )
    .fetch_one(&pool).await.unwrap();
    assert_eq!(leaks, 0);
    // 另一位顧客完全不受影響
    let hua: (String, Option<String>) =
        sqlx::query_as("SELECT name, phone FROM customers WHERE email = 'hua@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        hua,
        ("李小華".to_string(), Some("0988-111-222".to_string()))
    );

    // 後台:搜尋舊的姓名 / Email 找不到,但歷史統計還在(顯示「已刪除的顧客」)
    for q in ["%E7%8E%8B%E5%B0%8F%E6%98%8E", "ming%40example.com"] {
        let (_, list) = call(
            &s.app,
            Method::GET,
            &format!("/t/shop-a/customers?q={q}"),
            None,
            Some(&s.owner),
        )
        .await;
        assert!(list["items"].as_array().unwrap().is_empty(), "{q}: {list}");
    }
    let (_, detail) = call(
        &s.app,
        Method::GET,
        &format!("/t/shop-a/customers/{customer}"),
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(detail["name"], "已刪除的顧客");
    assert_eq!(detail["history"].as_array().unwrap().len(), 2);

    // 稽核:誰刪了哪位(id)、影響幾筆 —— 沒有個資
    let (action, entity, detail): (String, Option<Uuid>, Value) = sqlx::query_as(
        "SELECT action, entity_id, detail FROM audit_logs WHERE action = 'customer.anonymized'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (action.as_str(), entity),
        ("customer.anonymized", Some(customer))
    );
    // 沒寄出的信 3 封:預約時排的確認信、取消時排的取消通知,加上測試手動塞的那封
    assert_eq!(detail, json!({"bookings": 2, "unsent_mails_removed": 3}));

    // 同一個人之後再來預約:是全新的顧客,不會撞到唯一限制
    book(&s, 42, 7, "王小明", "ming@example.com", "0912-345-678").await;
    let again = customer_id(&pool, "ming@example.com").await;
    assert_ne!(again, customer);
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM customers")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 3);
}

#[sqlx::test]
async fn anonymizing_is_refused_while_the_customer_still_has_bookings_to_serve(pool: PgPool) {
    let s = shop(&pool).await;
    let id = book(&s, 3, 3, "王小明", "ming@example.com", "0912").await;
    let customer = customer_id(&pool, "ming@example.com").await;

    // 還有未來的已確認預約
    let (status, body) = anonymize(&s, &s.owner, customer).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().contains("1 筆"), "{body}");
    let name: String = sqlx::query_scalar("SELECT name FROM customers WHERE id = $1")
        .bind(customer)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "王小明", "被拒絕就什麼都沒改");

    // 待確認的申請也算(那位顧客正等著確認信)
    sqlx::query("UPDATE bookings SET status = 'pending' WHERE id = $1::uuid")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        anonymize(&s, &s.owner, customer).await.0,
        StatusCode::CONFLICT
    );

    // 取消之後可以;再刪一次是 409(已經刪過),不會重複處理
    call(
        &s.app,
        Method::PATCH,
        &format!("/t/shop-a/bookings/{id}"),
        Some(json!({"status": "cancelled"})),
        Some(&s.owner),
    )
    .await;
    assert_eq!(
        anonymize(&s, &s.owner, customer).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        anonymize(&s, &s.owner, customer).await.0,
        StatusCode::CONFLICT
    );
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = 'customer.anonymized'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test]
async fn only_the_owner_can_anonymize_and_never_across_shops(pool: PgPool) {
    let s = shop(&pool).await;
    book(&s, 3, 3, "王小明", "ming@example.com", "0912").await;
    let customer = customer_id(&pool, "ming@example.com").await;
    sqlx::query("UPDATE bookings SET status = 'cancelled'")
        .execute(&pool)
        .await
        .unwrap();

    let manager = signup(&s.app, "m@example.com").await;
    add_member(&pool, "shop-a", "m@example.com", "manager").await;
    let staff = signup(&s.app, "s@example.com").await;
    add_member(&pool, "shop-a", "s@example.com", "staff").await;
    let other = signup(&s.app, "b@example.com").await;
    create_tenant(&s.app, &other, "shop-b").await;

    // 不可逆的操作只有擁有者能做
    assert_eq!(
        anonymize(&s, &manager, customer).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        anonymize(&s, &staff, customer).await.0,
        StatusCode::FORBIDDEN
    );
    // 別家店的擁有者拿這個 id 打自己的店 → 找不到
    let (status, _) = call(
        &s.app,
        Method::POST,
        &format!("/t/shop-b/customers/{customer}/anonymize"),
        None,
        Some(&other),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        anonymize(&s, &s.owner, Uuid::new_v4()).await.0,
        StatusCode::NOT_FOUND
    );
    let name: String = sqlx::query_scalar("SELECT name FROM customers WHERE id = $1")
        .bind(customer)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "王小明");
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = 'customer.anonymized'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test]
async fn several_customers_can_be_erased_and_past_unfinished_bookings_do_not_block(pool: PgPool) {
    let s = shop(&pool).await;
    for (i, email) in ["a@example.com", "b@example.com"].into_iter().enumerate() {
        let id = book(&s, 40 + i as i64, 3, "顧客", email, "0912").await;
        call(
            &s.app,
            Method::PATCH,
            &format!("/t/shop-a/bookings/{id}"),
            Some(json!({"status": "cancelled"})),
            Some(&s.owner),
        )
        .await;
    }
    // 已經過去、卻還是「已確認」(店員忘了標完成):不是「未來的預約」,不該擋住刪除
    let c = customer_id(&pool, "a@example.com").await;
    sqlx::query(
        "INSERT INTO bookings (tenant_id, staff_user_id, service_id, customer_id, starts_at, ends_at,
                               status, manage_token_hash, confirmed_at)
         VALUES ($1, $2, $3, $4, now() - interval '5 days', now() - interval '5 days' + interval '1 hour',
                 'confirmed', md5(random()::text) || md5(random()::text), now() - interval '9 days')",
    )
    .bind(s.tenant_id)
    .bind(s.owner_id)
    .bind(s.service.parse::<Uuid>().unwrap())
    .bind(c)
    .execute(&pool)
    .await
    .unwrap();

    // 兩位都刪得掉(匿名 Email 各自唯一,不會撞到 (租戶, Email) 的唯一限制)
    let b = customer_id(&pool, "b@example.com").await;
    assert_eq!(anonymize(&s, &s.owner, c).await.0, StatusCode::NO_CONTENT);
    assert_eq!(anonymize(&s, &s.owner, b).await.0, StatusCode::NO_CONTENT);
    let emails: Vec<String> =
        sqlx::query_scalar("SELECT email::text FROM customers ORDER BY email")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(emails.len(), 2);
    assert!(emails.iter().all(|e| e.ends_with("@anonymized.invalid")));
    assert_ne!(emails[0], emails[1]);
}

/// 預約匯出的 `from` / `to` 是「與這段期間有重疊的預約」,不是「開始時間落在這段期間」:
/// 開始得比 `from` 早、但還沒結束的預約(例如跨過午夜的)也要包含,與列表的語意一致
#[sqlx::test]
async fn export_range_means_overlap_not_start_time(pool: PgPool) {
    let s = shop(&pool).await;
    // 這個測試檔的服務長 30 分鐘:預約是第 5 天的 03:00–03:30(UTC)
    book(&s, 5, 3, "甲", "a@example.com", "").await;
    let start = (Utc::now() + Duration::days(5))
        .format("%Y-%m-%dT03:00:00Z")
        .to_string()
        .parse::<chrono::DateTime<Utc>>()
        .unwrap();
    let iso = |t: chrono::DateTime<Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let count = |rows: &[Vec<String>]| rows.len() - 1;
    let to = iso(start + Duration::hours(2));

    // 期間從 03:10 開始:預約 03:00 開始、03:30 結束 → 重疊,要包含(開始得比 from 早,但還沒結束)
    let from = iso(start + Duration::minutes(10));
    let (_, rows, _) = export(&s, Some(&s.owner), &format!("?from={from}&to={to}")).await;
    assert_eq!(count(&rows), 1, "開始得比 from 早、還沒結束 → 包含");
    // 期間從 03:30 開始:預約剛好在 03:30 結束 → 沒有重疊(結束時間不含)
    let from = iso(start + Duration::minutes(30));
    let (_, rows, _) = export(&s, Some(&s.owner), &format!("?from={from}&to={to}")).await;
    assert_eq!(count(&rows), 0);
    // 期間在預約開始之前結束(to 不含):不重疊
    let from = iso(start - Duration::hours(2));
    let to = iso(start);
    let (_, rows, _) = export(&s, Some(&s.owner), &format!("?from={from}&to={to}")).await;
    assert_eq!(count(&rows), 0);
}

/// A 店刪除顧客,不能動到 B 店寄給同一個 Email 的信(兩家店各自的顧客資料互不相干)
#[sqlx::test]
async fn erasing_a_customer_never_touches_another_shops_mail(pool: PgPool) {
    let s = shop(&pool).await;
    let id = book(&s, 40, 3, "王小明", "ming@example.com", "0912").await;
    call(
        &s.app,
        Method::PATCH,
        &format!("/t/shop-a/bookings/{id}"),
        Some(json!({"status": "cancelled"})),
        Some(&s.owner),
    )
    .await;
    let other = signup(&s.app, "b@example.com").await;
    create_tenant(&s.app, &other, "shop-b").await;
    let tenant_b: Uuid = sqlx::query_scalar("SELECT id FROM tenants WHERE slug = 'shop-b'")
        .fetch_one(&pool)
        .await
        .unwrap();
    // B 店有一封寄給同一個 Email 的、還沒寄出的信
    sqlx::query("INSERT INTO email_outbox (tenant_id, to_email, subject, body, status) VALUES ($1, 'ming@example.com', 'B 店的信', 'hi', 'pending')")
        .bind(tenant_b)
        .execute(&pool)
        .await
        .unwrap();

    let customer = customer_id(&pool, "ming@example.com").await;
    assert_eq!(
        anonymize(&s, &s.owner, customer).await.0,
        StatusCode::NO_CONTENT
    );

    let theirs: Vec<(String, String)> =
        sqlx::query_as("SELECT to_email, status FROM email_outbox WHERE tenant_id = $1")
            .bind(tenant_b)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        theirs,
        [("ming@example.com".to_string(), "pending".to_string())],
        "B 店的信原封不動"
    );
}
