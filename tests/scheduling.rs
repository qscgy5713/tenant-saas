mod common;

use axum::http::{Method, StatusCode};
use common::{add_member, app, call, create_service, create_tenant, signup, user_id};
use serde_json::json;
use sqlx::PgPool;

/// 建立一家店,擁有者 a、員工 b;回傳 (app, owner token, staff token, owner id, staff id)
async fn setup(pool: PgPool) -> (axum::Router, String, String, uuid::Uuid, uuid::Uuid) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    let staff = signup(&app, "b@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    add_member(&pool, "shop-a", "b@example.com", "staff").await;
    let owner_id = user_id(&pool, "a@example.com").await;
    let staff_id = user_id(&pool, "b@example.com").await;
    (app, owner, staff, owner_id, staff_id)
}

#[sqlx::test]
async fn services_crud_and_permissions(pool: PgPool) {
    let (app, owner, staff, _, _) = setup(pool).await;

    let svc = create_service(&app, &owner, "shop-a", "剪髮").await;
    let id = svc["id"].as_str().unwrap();
    assert_eq!(svc["active"], true);

    // 員工可看、不可改
    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/t/shop-a/services/{id}"),
        None,
        Some(&staff),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        Method::POST,
        "/t/shop-a/services",
        Some(json!({"name": "x", "duration_minutes": 30})),
        Some(&staff),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/t/shop-a/services/{id}"),
        Some(json!({"name": "x"})),
        Some(&staff),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/t/shop-a/services/{id}"),
        None,
        Some(&staff),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 驗證
    for body in [
        json!({"name": "", "duration_minutes": 30}),
        json!({"name": "x", "duration_minutes": 3}),
        json!({"name": "x", "duration_minutes": 30, "price_cents": -1}),
    ] {
        let (status, _) = call(
            &app,
            Method::POST,
            "/t/shop-a/services",
            Some(body),
            Some(&owner),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // 局部更新:只改 active,其他欄位不變
    let (status, updated) = call(
        &app,
        Method::PATCH,
        &format!("/t/shop-a/services/{id}"),
        Some(json!({"active": false})),
        Some(&owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["active"], false);
    assert_eq!(updated["name"], "剪髮");
    assert_eq!(updated["price_cents"], 500);

    let (_, active) = call(
        &app,
        Method::GET,
        "/t/shop-a/services?active=true",
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(active["total"], 0);
    let (_, all) = call(&app, Method::GET, "/t/shop-a/services", None, Some(&owner)).await;
    assert_eq!(all["total"], 1);

    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/t/shop-a/services/{id}"),
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/t/shop-a/services/{id}"),
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn services_pagination(pool: PgPool) {
    let (app, owner, _, _, _) = setup(pool).await;
    for name in ["a", "b", "c"] {
        create_service(&app, &owner, "shop-a", name).await;
    }
    let (_, page) = call(
        &app,
        Method::GET,
        "/t/shop-a/services?limit=2",
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(
        (
            page["items"].as_array().unwrap().len(),
            page["total"].as_i64().unwrap()
        ),
        (2, 3)
    );
    let (_, page) = call(
        &app,
        Method::GET,
        "/t/shop-a/services?limit=2&offset=2",
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    // 超大 limit 會被夾住
    let (_, page) = call(
        &app,
        Method::GET,
        "/t/shop-a/services?limit=100000",
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(page["limit"], 100);
}

#[sqlx::test]
async fn services_are_isolated_even_for_a_user_in_both_tenants(pool: PgPool) {
    let app = app(pool);
    let user = signup(&app, "a@example.com").await;
    create_tenant(&app, &user, "shop-a").await;
    create_tenant(&app, &user, "shop-b").await;
    let in_a = create_service(&app, &user, "shop-a", "只屬於 A").await;
    let id = in_a["id"].as_str().unwrap();

    // 同一個人、同一個 token,但在 B 店的上下文裡看不到 A 店的資料
    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/t/shop-b/services/{id}"),
        None,
        Some(&user),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/t/shop-b/services/{id}"),
        Some(json!({"name": "hacked"})),
        Some(&user),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/t/shop-b/services/{id}"),
        None,
        Some(&user),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list) = call(&app, Method::GET, "/t/shop-b/services", None, Some(&user)).await;
    assert_eq!(list["total"], 0);

    // A 店的資料沒被動到
    let (_, a) = call(
        &app,
        Method::GET,
        &format!("/t/shop-a/services/{id}"),
        None,
        Some(&user),
    )
    .await;
    assert_eq!(a["name"], "只屬於 A");
}

#[sqlx::test]
async fn staff_services_assignment(pool: PgPool) {
    let (app, owner, staff, _, staff_id) = setup(pool).await;
    let s1 = create_service(&app, &owner, "shop-a", "剪髮").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let s2 = create_service(&app, &owner, "shop-a", "染髮").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let uri = format!("/t/shop-a/members/{staff_id}/services");

    let (status, body) = call(
        &app,
        Method::PUT,
        &uri,
        Some(json!({"service_ids": [s1, s2, s1]})),
        Some(&owner),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["service_ids"].as_array().unwrap().len(),
        2,
        "重複的 id 只算一次"
    );

    // 取代而不是累加
    let (_, body) = call(
        &app,
        Method::PUT,
        &uri,
        Some(json!({"service_ids": [s2]})),
        Some(&owner),
    )
    .await;
    assert_eq!(body["service_ids"], json!([s2]));

    // 員工不能改自己的服務清單
    let (status, _) = call(
        &app,
        Method::PUT,
        &uri,
        Some(json!({"service_ids": []})),
        Some(&staff),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 別家店的服務、不存在的服務:400,而且原本的設定不變
    let other = signup(&app, "c@example.com").await;
    create_tenant(&app, &other, "shop-c").await;
    let foreign = create_service(&app, &other, "shop-c", "別家").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, _) = call(
        &app,
        Method::PUT,
        &uri,
        Some(json!({"service_ids": [s1, foreign]})),
        Some(&owner),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, body) = call(&app, Method::GET, &uri, None, Some(&owner)).await;
    assert_eq!(body["service_ids"], json!([s2]), "失敗的更新必須整批還原");

    // 不是這家店的成員
    let stranger = uuid::Uuid::new_v4();
    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/t/shop-a/members/{stranger}/services"),
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn working_hours(pool: PgPool) {
    let (app, owner, staff, owner_id, staff_id) = setup(pool).await;
    let own = format!("/t/shop-a/members/{staff_id}/working-hours");
    let hours = |h: serde_json::Value| json!({"hours": h});

    // 員工改自己
    let good = hours(json!([
        {"weekday": 1, "start": "09:00", "end": "12:00"},
        {"weekday": 1, "start": "13:00", "end": "18:00"},
        {"weekday": 2, "start": "09:00", "end": "18:00"},
    ]));
    let (status, body) = call(&app, Method::PUT, &own, Some(good.clone()), Some(&staff)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["hours"].as_array().unwrap().len(), 3);
    assert_eq!(body["hours"][0]["start"], "09:00");

    // 失敗的更新不能清掉舊資料
    for bad in [
        hours(
            json!([{"weekday": 1, "start": "09:00", "end": "12:00"}, {"weekday": 1, "start": "11:00", "end": "13:00"}]),
        ),
        hours(json!([{"weekday": 1, "start": "12:00", "end": "09:00"}])),
        hours(json!([{"weekday": 7, "start": "09:00", "end": "12:00"}])),
        hours(json!([{"weekday": 1, "start": "9am", "end": "12:00"}])),
    ] {
        let (status, _) = call(&app, Method::PUT, &own, Some(bad), Some(&staff)).await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::CONFLICT,
            "{status}"
        );
    }
    let (_, body) = call(&app, Method::GET, &own, None, Some(&staff)).await;
    assert_eq!(body["hours"].as_array().unwrap().len(), 3);

    // 相鄰(12:00 結束、12:00 開始)不算重疊
    let adjacent = hours(
        json!([{"weekday": 3, "start": "09:00", "end": "12:00"}, {"weekday": 3, "start": "12:00", "end": "15:00"}]),
    );
    assert_eq!(
        call(&app, Method::PUT, &own, Some(adjacent), Some(&staff))
            .await
            .0,
        StatusCode::OK
    );

    // 重疊回 409
    let overlap = hours(
        json!([{"weekday": 3, "start": "09:00", "end": "12:00"}, {"weekday": 3, "start": "11:00", "end": "15:00"}]),
    );
    assert_eq!(
        call(&app, Method::PUT, &own, Some(overlap), Some(&staff))
            .await
            .0,
        StatusCode::CONFLICT
    );

    // 員工不能改別人;管理者可以改員工
    let owners = format!("/t/shop-a/members/{owner_id}/working-hours");
    assert_eq!(
        call(&app, Method::PUT, &owners, Some(good.clone()), Some(&staff))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            Method::PUT,
            &own,
            Some(hours(json!([]))),
            Some(&owner)
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, body) = call(&app, Method::GET, &own, None, Some(&owner)).await;
    assert_eq!(body["hours"], json!([]));
}

#[sqlx::test]
async fn time_off(pool: PgPool) {
    let (app, owner, staff, owner_id, staff_id) = setup(pool.clone()).await;
    let own = format!("/t/shop-a/members/{staff_id}/time-off");
    let range = json!({"starts_at": "2026-12-24T00:00:00Z", "ends_at": "2026-12-26T00:00:00Z", "reason": "旅行"});

    let (status, created) = call(&app, Method::POST, &own, Some(range.clone()), Some(&staff)).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap();

    let bad = json!({"starts_at": "2026-12-26T00:00:00Z", "ends_at": "2026-12-24T00:00:00Z"});
    assert_eq!(
        call(&app, Method::POST, &own, Some(bad), Some(&staff))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );

    // 員工不能替別人請假,也不能刪別人的
    let owners = format!("/t/shop-a/members/{owner_id}/time-off");
    assert_eq!(
        call(
            &app,
            Method::POST,
            &owners,
            Some(range.clone()),
            Some(&staff)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (_, theirs) = call(&app, Method::POST, &owners, Some(range), Some(&owner)).await;
    let their_id = theirs["id"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/t/shop-a/time-off/{their_id}"),
            None,
            Some(&staff)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    let (_, list) = call(&app, Method::GET, &own, None, Some(&staff)).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["reason"], "旅行", "本人看得到自己的原因");

    // 同事(非主管)只看得到時段,看不到原因;主管看得到
    let coworker = signup(&app, "d@example.com").await;
    add_member(&pool, "shop-a", "d@example.com", "staff").await;
    let (_, seen) = call(&app, Method::GET, &own, None, Some(&coworker)).await;
    assert_eq!(seen.as_array().unwrap().len(), 1);
    assert!(seen[0]["reason"].is_null());
    let (_, seen) = call(&app, Method::GET, &own, None, Some(&owner)).await;
    assert_eq!(seen[0]["reason"], "旅行");
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/t/shop-a/time-off/{id}"),
            None,
            Some(&staff)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/t/shop-a/time-off/{id}"),
            None,
            Some(&staff)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}
