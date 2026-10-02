mod common;

use axum::{
    Router,
    http::{Method, StatusCode},
};
use common::{add_member, app, call, create_tenant, set_plan, signup, user_id};
use serde_json::{Value, json};
use sqlx::PgPool;

async fn invite(
    app: &Router,
    token: &str,
    slug: &str,
    email: &str,
    role: &str,
) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        &format!("/t/{slug}/invitations"),
        Some(json!({"email": email, "role": role})),
        Some(token),
    )
    .await
}

async fn accept(app: &Router, token: &str, invite_token: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/invitations/accept",
        Some(json!({"token": invite_token})),
        Some(token),
    )
    .await
}

#[sqlx::test]
async fn invite_and_accept_flow(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    let invitee = signup(&app, "b@example.com").await;
    let other = signup(&app, "c@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;

    let (status, inv) = invite(&app, &owner, "shop-a", "B@Example.com", "staff").await;
    assert_eq!(status, StatusCode::CREATED);
    let token = inv["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 64);

    // 資料庫存的是雜湊,不是原始 token
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM invitations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(stored, token);
    assert_eq!(stored.len(), 64);

    // 別人拿到連結也不能用(Email 不符),且回應與「無效 token」相同
    let wrong_user = accept(&app, &other, &token).await;
    let garbage = accept(&app, &other, &"0".repeat(64)).await;
    assert_eq!(wrong_user.0, StatusCode::NOT_FOUND);
    assert_eq!(wrong_user, garbage);
    assert_eq!(
        call(
            &app,
            Method::POST,
            "/invitations/accept",
            Some(json!({"token": token})),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );

    // 本人接受
    let (status, body) = accept(&app, &invitee, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tenant_slug"], "shop-a");
    let (_, me) = call(&app, Method::GET, "/t/shop-a/me", None, Some(&invitee)).await;
    assert_eq!(me["role"], "staff");

    // 邀請只能用一次
    assert_eq!(
        accept(&app, &invitee, &token).await.0,
        StatusCode::NOT_FOUND
    );
    // 已經是成員,不能再邀請
    assert_eq!(
        invite(&app, &owner, "shop-a", "b@example.com", "staff")
            .await
            .0,
        StatusCode::CONFLICT
    );
}

#[sqlx::test]
async fn invite_permissions_and_validation(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    let manager = signup(&app, "m@example.com").await;
    let staff = signup(&app, "s@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    set_plan(&pool, "shop-a", "business").await; // 免費版人數上限 2,這個測試要邀請很多人
    add_member(&pool, "shop-a", "m@example.com", "manager").await;
    add_member(&pool, "shop-a", "s@example.com", "staff").await;

    assert_eq!(
        invite(&app, &staff, "shop-a", "x@example.com", "staff")
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        invite(&app, &manager, "shop-a", "x@example.com", "manager")
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        invite(&app, &manager, "shop-a", "x@example.com", "staff")
            .await
            .0,
        StatusCode::CREATED
    );
    assert_eq!(
        invite(&app, &owner, "shop-a", "y@example.com", "manager")
            .await
            .0,
        StatusCode::CREATED
    );

    assert_eq!(
        invite(&app, &owner, "shop-a", "x@example.com", "staff")
            .await
            .0,
        StatusCode::CONFLICT,
        "同 Email 不可重複邀請"
    );
    assert_eq!(
        invite(&app, &owner, "shop-a", "z@example.com", "owner")
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        invite(&app, &owner, "shop-a", "not-an-email", "staff")
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let (status, _) = call(
        &app,
        Method::POST,
        "/t/shop-a/invitations",
        Some(json!({"email": "q@example.com", "role": "boss"})),
        Some(&owner),
    )
    .await;
    assert!(status.is_client_error());

    // 員工看不到邀請清單
    assert_eq!(
        call(
            &app,
            Method::GET,
            "/t/shop-a/invitations",
            None,
            Some(&staff)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (_, list) = call(
        &app,
        Method::GET,
        "/t/shop-a/invitations",
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert!(list[0].get("token").is_none() && list[0].get("token_hash").is_none());
}

#[sqlx::test]
async fn expired_and_revoked_invitations(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    let manager = signup(&app, "m@example.com").await;
    let invitee = signup(&app, "b@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    set_plan(&pool, "shop-a", "business").await; // 免費版人數上限 2,這個測試要邀請很多人
    add_member(&pool, "shop-a", "m@example.com", "manager").await;

    // 過期
    let (_, inv) = invite(&app, &owner, "shop-a", "b@example.com", "staff").await;
    let old = inv["token"].as_str().unwrap().to_string();
    sqlx::query("UPDATE invitations SET expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(accept(&app, &invitee, &old).await.0, StatusCode::NOT_FOUND);
    let (_, list) = call(
        &app,
        Method::GET,
        "/t/shop-a/invitations",
        None,
        Some(&owner),
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 0, "過期的不算待處理");

    // 過期後可以重新邀請,舊 token 仍然無效
    let (status, fresh) = invite(&app, &owner, "shop-a", "b@example.com", "staff").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(accept(&app, &invitee, &old).await.0, StatusCode::NOT_FOUND);

    // 撤銷後 token 失效;manager 不能撤銷邀請 manager 的邀請
    let id = fresh["id"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/t/shop-a/invitations/{id}"),
            None,
            Some(&manager)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        accept(&app, &invitee, fresh["token"].as_str().unwrap())
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let (_, mgr_inv) = invite(&app, &owner, "shop-a", "n@example.com", "manager").await;
    let mid = mgr_inv["id"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/t/shop-a/invitations/{mid}"),
            None,
            Some(&manager)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/t/shop-a/invitations/{mid}"),
            None,
            Some(&owner)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test]
async fn invitations_are_isolated_between_tenants(pool: PgPool) {
    let app = app(pool);
    let user = signup(&app, "a@example.com").await;
    create_tenant(&app, &user, "shop-a").await;
    create_tenant(&app, &user, "shop-b").await;
    let (_, inv) = invite(&app, &user, "shop-a", "x@example.com", "staff").await;
    let id = inv["id"].as_str().unwrap();

    let (_, list_b) = call(
        &app,
        Method::GET,
        "/t/shop-b/invitations",
        None,
        Some(&user),
    )
    .await;
    assert_eq!(list_b.as_array().unwrap().len(), 0);
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/t/shop-b/invitations/{id}"),
            None,
            Some(&user)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, list_a) = call(
        &app,
        Method::GET,
        "/t/shop-a/invitations",
        None,
        Some(&user),
    )
    .await;
    assert_eq!(list_a.as_array().unwrap().len(), 1);
}

#[sqlx::test]
async fn change_role_and_remove_members(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    let m1 = signup(&app, "m1@example.com").await;
    let m2 = signup(&app, "m2@example.com").await;
    let s1 = signup(&app, "s1@example.com").await;
    signup(&app, "s2@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    for (email, role) in [
        ("m1@example.com", "manager"),
        ("m2@example.com", "manager"),
        ("s1@example.com", "staff"),
        ("s2@example.com", "staff"),
    ] {
        add_member(&pool, "shop-a", email, role).await;
    }
    let (owner_id, m2_id, s1_id, s2_id) = (
        user_id(&pool, "a@example.com").await,
        user_id(&pool, "m2@example.com").await,
        user_id(&pool, "s1@example.com").await,
        user_id(&pool, "s2@example.com").await,
    );
    let member = |id: uuid::Uuid| format!("/t/shop-a/members/{id}");
    let set_role = |token: &str, id: uuid::Uuid, role: &str| {
        let (app, token, role) = (app.clone(), token.to_string(), role.to_string());
        async move {
            call(
                &app,
                Method::PATCH,
                &member(id),
                Some(json!({"role": role})),
                Some(&token),
            )
            .await
        }
    };

    // 角色調整:只有 owner
    assert_eq!(
        set_role(&m1, s1_id, "manager").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        set_role(&s1, s1_id, "manager").await.0,
        StatusCode::FORBIDDEN,
        "不能自己升級自己"
    );
    assert_eq!(
        set_role(&owner, s1_id, "owner").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        set_role(&owner, owner_id, "staff").await.0,
        StatusCode::CONFLICT,
        "owner 不能被降級"
    );
    assert_eq!(
        set_role(&owner, uuid::Uuid::new_v4(), "staff").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(set_role(&owner, s1_id, "manager").await.0, StatusCode::OK);
    assert_eq!(set_role(&owner, s1_id, "staff").await.0, StatusCode::OK);

    // 移除:員工不能移除別人;manager 不能移除 manager 或 owner;owner 不可被移除
    let del = |token: &str, id: uuid::Uuid| {
        let (app, token) = (app.clone(), token.to_string());
        async move {
            call(&app, Method::DELETE, &member(id), None, Some(&token))
                .await
                .0
        }
    };
    assert_eq!(del(&s1, s2_id).await, StatusCode::FORBIDDEN);
    assert_eq!(del(&m1, m2_id).await, StatusCode::FORBIDDEN);
    assert_eq!(del(&m1, owner_id).await, StatusCode::CONFLICT);
    assert_eq!(
        del(&owner, owner_id).await,
        StatusCode::CONFLICT,
        "owner 不能自己退出"
    );
    assert_eq!(del(&m1, s2_id).await, StatusCode::NO_CONTENT);

    // 移除成員會連帶清掉他的排程資料
    call(
        &app,
        Method::PUT,
        &format!("/t/shop-a/members/{s1_id}/working-hours"),
        Some(json!({"hours": [{"weekday": 1, "start": "09:00", "end": "12:00"}]})),
        Some(&owner),
    )
    .await;
    assert_eq!(del(&owner, s1_id).await, StatusCode::NO_CONTENT);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM working_hours")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
    assert_eq!(
        call(&app, Method::GET, "/t/shop-a/me", None, Some(&s1))
            .await
            .0,
        StatusCode::NOT_FOUND,
        "被移除後立即失去存取"
    );

    // 自己退出
    assert_eq!(del(&m2, m2_id).await, StatusCode::NO_CONTENT);
    assert_eq!(
        call(&app, Method::GET, "/t/shop-a/me", None, Some(&m2))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        del(&owner, s2_id).await,
        StatusCode::NOT_FOUND,
        "已經不是成員"
    );
}
