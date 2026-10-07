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

// ---------- 停用(離職) ----------

async fn post_member(
    app: &Router,
    token: &str,
    user: uuid::Uuid,
    action: &str,
) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        &format!("/t/shop-a/members/{user}/{action}"),
        None,
        Some(token),
    )
    .await
}

async fn member_list(app: &Router, token: &str) -> Vec<Value> {
    let (status, body) = call(app, Method::GET, "/t/shop-a/members", None, Some(token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.as_array().unwrap().clone()
}

#[sqlx::test]
async fn deactivated_member_loses_access_but_history_stays(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    let staff = signup(&app, "b@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    add_member(&pool, "shop-a", "b@example.com", "staff").await;
    let b = user_id(&pool, "b@example.com").await;

    let me = |t: &str| {
        let (app, t) = (app.clone(), t.to_string());
        async move {
            call(&app, Method::GET, "/t/shop-a/me", None, Some(&t))
                .await
                .0
        }
    };
    assert_eq!(me(&staff).await, StatusCode::OK);

    assert_eq!(
        post_member(&app, &owner, b, "deactivate").await.0,
        StatusCode::NO_CONTENT
    );

    // 進不了這間店,店家也不在他的清單裡 —— 但帳號還在(其他店不受影響)
    assert_eq!(me(&staff).await, StatusCode::NOT_FOUND);
    let (_, shops) = call(&app, Method::GET, "/tenants", None, Some(&staff)).await;
    assert!(shops.as_array().unwrap().is_empty(), "{shops}");
    assert_eq!(
        call(&app, Method::GET, "/auth/me", None, Some(&staff))
            .await
            .0,
        StatusCode::OK
    );

    // 成員清單仍列出他(標示停用,排在啟用的後面),稽核有紀錄
    let list = member_list(&app, &owner).await;
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["email"], "a@example.com");
    assert_eq!(
        (list[1]["email"].as_str(), list[1]["active"].as_bool()),
        (Some("b@example.com"), Some(false))
    );
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = 'member.deactivated'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 1);

    // 重新啟用 → 回來了
    assert_eq!(
        post_member(&app, &owner, b, "reactivate").await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(me(&staff).await, StatusCode::OK);
    assert_eq!(member_list(&app, &owner).await[1]["active"], true);
}

#[sqlx::test]
async fn who_may_deactivate_whom(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    let manager = signup(&app, "m@example.com").await;
    signup(&app, "m2@example.com").await;
    let staff = signup(&app, "s@example.com").await;
    let staff2 = signup(&app, "s2@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    set_plan(&pool, "shop-a", "business").await; // 名額不是這個測試要測的
    for (email, role) in [
        ("m@example.com", "manager"),
        ("m2@example.com", "manager"),
        ("s@example.com", "staff"),
        ("s2@example.com", "staff"),
    ] {
        add_member(&pool, "shop-a", email, role).await;
    }
    let id = |e: &'static str| {
        let pool = pool.clone();
        async move { user_id(&pool, e).await }
    };
    let (a, m2, s, s2) = (
        id("a@example.com").await,
        id("m2@example.com").await,
        id("s@example.com").await,
        id("s2@example.com").await,
    );

    // 擁有者不能被停用(誰都不行,包括自己)
    assert_eq!(
        post_member(&app, &owner, a, "deactivate").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        post_member(&app, &manager, a, "deactivate").await.0,
        StatusCode::CONFLICT
    );
    // 員工不能停用別人;管理者不能停用同階的管理者
    assert_eq!(
        post_member(&app, &staff, s2, "deactivate").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post_member(&app, &manager, m2, "deactivate").await.0,
        StatusCode::FORBIDDEN
    );
    // 不存在的成員
    assert_eq!(
        post_member(&app, &owner, uuid::Uuid::new_v4(), "deactivate")
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    // 管理者可以停用員工;員工可以停用自己(退出)
    assert_eq!(
        post_member(&app, &manager, s, "deactivate").await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post_member(&app, &staff2, s2, "deactivate").await.0,
        StatusCode::NO_CONTENT
    );
    // 重複停用
    assert_eq!(
        post_member(&app, &owner, s, "deactivate").await.0,
        StatusCode::CONFLICT
    );

    // 重新啟用:只有管理者以上;員工自己停用了就進不來,也不能自己回來
    assert_eq!(
        post_member(&app, &staff2, s2, "reactivate").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post_member(&app, &manager, m2, "reactivate").await.0,
        StatusCode::FORBIDDEN,
        "同階的管理者不能動"
    );
    assert_eq!(
        post_member(&app, &owner, m2, "reactivate").await.0,
        StatusCode::CONFLICT,
        "本來就是啟用狀態"
    );
    assert_eq!(
        post_member(&app, &manager, s, "reactivate").await.0,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test]
async fn deactivated_members_do_not_use_up_plan_seats(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    let _b = signup(&app, "b@example.com").await;
    let c = signup(&app, "c@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    add_member(&pool, "shop-a", "b@example.com", "staff").await;
    let b_id = user_id(&pool, "b@example.com").await;
    // 免費版 2 人:擁有者 + b 已滿
    assert_eq!(
        invite(&app, &owner, "shop-a", "c@example.com", "staff")
            .await
            .0,
        StatusCode::PAYMENT_REQUIRED
    );

    // 停用 b → 空出一個名額
    post_member(&app, &owner, b_id, "deactivate").await;
    let (status, inv) = invite(&app, &owner, "shop-a", "c@example.com", "staff").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        accept(&app, &c, inv["token"].as_str().unwrap()).await.0,
        StatusCode::OK,
        "接受邀請時也只算啟用中的成員"
    );

    // 名額被 c 用掉了 → b 不能直接回來
    let (status, _) = post_member(&app, &owner, b_id, "reactivate").await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    // 方案頁的用量也只算啟用中的
    let (_, plan) = call(&app, Method::GET, "/t/shop-a/plan", None, Some(&owner)).await;
    assert_eq!(plan["usage"]["staff"], 2);

    // 升級後就可以
    set_plan(&pool, "shop-a", "pro").await;
    assert_eq!(
        post_member(&app, &owner, b_id, "reactivate").await.0,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test]
async fn deactivated_members_cannot_be_invited_or_promoted_and_delete_points_to_deactivate(
    pool: PgPool,
) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    signup(&app, "b@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    set_plan(&pool, "shop-a", "business").await;
    add_member(&pool, "shop-a", "b@example.com", "staff").await;
    let b = user_id(&pool, "b@example.com").await;
    post_member(&app, &owner, b, "deactivate").await;

    // 再邀請他 → 告訴你該按「重新啟用」,不是含糊的「已經是成員」
    let (status, body) = invite(&app, &owner, "shop-a", "b@example.com", "staff").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"].as_str().unwrap().contains("重新啟用"),
        "{body}"
    );

    // 停用中不能改角色
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/t/shop-a/members/{b}"),
        Some(json!({"role": "manager"})),
        Some(&owner),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

// ---------- 移交店主 ----------

async fn transfer(
    app: &Router,
    token: &str,
    to: uuid::Uuid,
    password: &str,
) -> (StatusCode, Value) {
    call(
        app,
        Method::POST,
        "/t/shop-a/transfer-ownership",
        Some(json!({"user_id": to, "password": password})),
        Some(token),
    )
    .await
}

async fn role_of(pool: &PgPool, email: &str) -> String {
    sqlx::query_scalar(
        "SELECT m.role::text FROM memberships m JOIN users u ON u.id = m.user_id WHERE u.email = $1::citext",
    )
    .bind(email)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn the_owner_can_hand_the_shop_to_a_member_and_becomes_a_manager(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    let manager = signup(&app, "m@example.com").await;
    let staff = signup(&app, "s@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    add_member(&pool, "shop-a", "m@example.com", "manager").await;
    add_member(&pool, "shop-a", "s@example.com", "staff").await;
    let m = user_id(&pool, "m@example.com").await;
    let a = user_id(&pool, "a@example.com").await;

    // 只有店主、不能給自己、密碼要對
    assert_eq!(
        transfer(&app, &manager, m, "password123").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        transfer(&app, &staff, m, "password123").await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        transfer(&app, &owner, a, "password123").await.0,
        StatusCode::BAD_REQUEST
    );
    let (status, body) = transfer(&app, &owner, m, "wrong-password").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(role_of(&pool, "a@example.com").await, "owner");
    // 不存在 / 別家店的人
    assert_eq!(
        transfer(&app, &owner, uuid::Uuid::new_v4(), "password123")
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let (status, body) = transfer(&app, &owner, m, "password123").await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(role_of(&pool, "m@example.com").await, "owner");
    assert_eq!(role_of(&pool, "a@example.com").await, "manager");
    // 店家永遠只有一位擁有者
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memberships WHERE role = 'owner'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    // 稽核、通知新店主
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_logs WHERE action = 'ownership.transferred'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    let (subject, body): (String, String) = sqlx::query_as(
        "SELECT subject, body FROM email_outbox WHERE dedupe_key LIKE 'ownership_transferred:%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        subject.contains("擁有者") && body.contains("小明"),
        "{subject} {body}"
    );

    // 權限立刻跟著變:舊店主不能再改店家設定,新店主可以
    let (status, _) = call(
        &app,
        Method::PATCH,
        "/t/shop-a",
        Some(json!({"name": "新名字"})),
        Some(&owner),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        Method::PATCH,
        "/t/shop-a",
        Some(json!({"name": "新名字"})),
        Some(&manager),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // 舊店主現在可以刪除自己的帳號(不再是店主)
    let (status, _) = call(
        &app,
        Method::POST,
        "/auth/delete-account",
        Some(json!({"password": "password123"})),
        Some(&owner),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test]
async fn ownership_cannot_go_to_someone_deactivated_or_already_an_owner(pool: PgPool) {
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    signup(&app, "m@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    add_member(&pool, "shop-a", "m@example.com", "manager").await;
    let m = user_id(&pool, "m@example.com").await;
    let a = user_id(&pool, "a@example.com").await;

    let (status, _) = post_member(&app, &owner, m, "deactivate").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = transfer(&app, &owner, m, "password123").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(role_of(&pool, "a@example.com").await, "owner");
    let _ = a;
}

#[sqlx::test]
async fn ownership_cannot_go_to_someone_who_already_owns_the_shop(pool: PgPool) {
    // 正常情況下店家只有一位擁有者;這裡用 SQL 造出「另一位也是擁有者」的狀態,確認移交會拒絕
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    signup(&app, "m@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    add_member(&pool, "shop-a", "m@example.com", "manager").await;
    sqlx::query("UPDATE memberships SET role = 'owner' WHERE user_id = (SELECT id FROM users WHERE email = 'm@example.com')")
        .execute(&pool)
        .await
        .unwrap();
    let m = user_id(&pool, "m@example.com").await;
    let (status, body) = transfer(&app, &owner, m, "password123").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(role_of(&pool, "a@example.com").await, "owner");
}

#[sqlx::test]
async fn wrong_passwords_when_transferring_count_towards_the_account_lockout(pool: PgPool) {
    // 持有被盜登入狀態的人不能靠這個端點無限次猜密碼
    let app = app(pool.clone());
    let owner = signup(&app, "a@example.com").await;
    signup(&app, "m@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    add_member(&pool, "shop-a", "m@example.com", "manager").await;
    let m = user_id(&pool, "m@example.com").await;

    for _ in 0..4 {
        assert_eq!(
            transfer(&app, &owner, m, "wrong-password").await.0,
            StatusCode::BAD_REQUEST
        );
    }
    // 第 5 次失敗 → 鎖定、寄通知信
    assert_eq!(
        transfer(&app, &owner, m, "wrong-password").await.0,
        StatusCode::BAD_REQUEST
    );
    let (status, body) = transfer(&app, &owner, m, "password123").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "鎖定中連正確密碼都不收: {body}"
    );
    assert!(body["error"].as_str().unwrap().contains("鎖定"), "{body}");
    assert_eq!(role_of(&pool, "a@example.com").await, "owner");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM email_outbox WHERE to_email = 'a@example.com' AND subject LIKE '%鎖定%'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    // 帳號被鎖:登入也被擋(和猜登入密碼一致)
    let (status, _) = call(
        &app,
        Method::POST,
        "/auth/login",
        Some(json!({"email": "a@example.com", "password": "password123"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 鎖定到期後正確的密碼可以;成功會清掉先前的失敗次數
    sqlx::query("UPDATE users SET locked_until = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        transfer(&app, &owner, m, "password123").await.0,
        StatusCode::NO_CONTENT
    );
    let failed: i32 =
        sqlx::query_scalar("SELECT failed_logins FROM users WHERE email = 'a@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(failed, 0);
}

#[sqlx::test]
async fn transferring_back_and_forth_notifies_the_new_owner_every_time(pool: PgPool) {
    // 去重鍵若固定為「店家 + 舊店主 + 新店主」,A → B → A → B 的第二次通知會被吞掉
    let app = app(pool.clone());
    let a_token = signup(&app, "a@example.com").await;
    let m_token = signup(&app, "m@example.com").await;
    create_tenant(&app, &a_token, "shop-a").await;
    add_member(&pool, "shop-a", "m@example.com", "manager").await;
    let a = user_id(&pool, "a@example.com").await;
    let m = user_id(&pool, "m@example.com").await;

    for (token, to) in [(&a_token, m), (&m_token, a), (&a_token, m)] {
        let (status, body) = transfer(&app, token, to, "password123").await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }
    let to_m: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM email_outbox
         WHERE dedupe_key LIKE 'ownership_transferred:%' AND to_email = 'm@example.com'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(to_m, 2);
}
