//! 瀏覽器登入:HttpOnly cookie + Origin 檢查(CSRF)。

mod common;

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use common::{app, call, register, test_config};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tenant_saas::{
    config::Config,
    routes::{self, AppState},
};
use tower::ServiceExt;

const ORIGIN: &str = "http://app.test";

async fn raw(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    headers: &[(header::HeaderName, &str)],
) -> (StatusCode, HeaderMap, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        req = req.header(k, *v);
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn set_cookie(headers: &HeaderMap) -> String {
    headers
        .get(header::SET_COOKIE)
        .expect("應該有 Set-Cookie")
        .to_str()
        .unwrap()
        .to_string()
}

/// `session=<jwt>; Path=/; …` → `session=<jwt>`(瀏覽器之後帶回來的部分)
fn cookie_pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().to_string()
}

async fn login(app: &Router, email: &str) -> (String, Value) {
    let (status, headers, body) = raw(
        app,
        Method::POST,
        "/auth/login",
        Some(json!({"email": email, "password": "password123"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    (cookie_pair(&set_cookie(&headers)), body)
}

async fn setup(pool: PgPool) -> (Router, String) {
    let app = app(pool);
    register(&app, "a@example.com", "password123").await;
    let (cookie, _) = login(&app, "a@example.com").await;
    (app, cookie)
}

fn new_shop(slug: &str) -> Value {
    json!({"slug": slug, "name": "店", "timezone": "Asia/Taipei"})
}

#[sqlx::test]
async fn login_and_register_set_a_protected_cookie(pool: PgPool) {
    let app = app(pool);
    let (status, headers, body) = raw(
        &app,
        Method::POST,
        "/auth/register",
        Some(json!({"email": "a@example.com", "password": "password123", "name": "甲"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cookie = set_cookie(&headers);
    for attr in ["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=3600"] {
        assert!(cookie.contains(attr), "{attr} 不在 {cookie}");
    }
    assert!(cookie.starts_with("session="), "{cookie}");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    // body 的 token 是給非瀏覽器客戶端用的,與 cookie 是同一個
    assert_eq!(
        cookie_pair(&cookie),
        format!("session={}", body["token"].as_str().unwrap())
    );

    let (_, headers, _) = raw(
        &app,
        Method::POST,
        "/auth/login",
        Some(json!({"email": "a@example.com", "password": "password123"})),
        &[],
    )
    .await;
    assert!(set_cookie(&headers).contains("HttpOnly"));
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");

    // 登入失敗不能發 cookie
    let (status, headers, _) = raw(
        &app,
        Method::POST,
        "/auth/login",
        Some(json!({"email": "a@example.com", "password": "wrong-password"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(headers.get(header::SET_COOKIE).is_none());
}

#[sqlx::test]
async fn production_uses_host_prefix_and_secure(pool: PgPool) {
    let config = Config {
        production: true,
        ..test_config(3600)
    };
    let app = routes::router(AppState::new(pool, &config));
    register(&app, "a@example.com", "password123").await;
    let (_, headers, _) = raw(
        &app,
        Method::POST,
        "/auth/login",
        Some(json!({"email": "a@example.com", "password": "password123"})),
        &[],
    )
    .await;
    let cookie = set_cookie(&headers);
    assert!(cookie.starts_with("__Host-session="), "{cookie}");
    assert!(cookie.contains("; Secure"), "{cookie}");
    assert!(!cookie.contains("Domain"), "{cookie}");
}

#[sqlx::test]
async fn cookie_authenticates_reads_without_origin(pool: PgPool) {
    let (app, cookie) = setup(pool).await;
    let (status, _, body) = raw(
        &app,
        Method::GET,
        "/auth/me",
        None,
        &[(header::COOKIE, &cookie)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], "a@example.com");
}

#[sqlx::test]
async fn cookie_writes_need_an_allowed_origin(pool: PgPool) {
    let (app, cookie) = setup(pool.clone()).await;
    let shops = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tenants")
            .fetch_one(&pool)
            .await
            .unwrap()
    };

    // 沒有 Origin、別的網站、同網域的其他子網域、前綴相同的網域 → 全部 403,而且什麼都沒發生
    for (i, origin) in [
        None,
        Some("http://evil.test"),
        Some("http://blog.app.test"),
        Some("http://app.test.evil.io"),
        Some("null"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut h = vec![(header::COOKIE, cookie.as_str())];
        if let Some(o) = origin {
            h.push((header::ORIGIN, o));
        }
        let (status, _, _) = raw(
            &app,
            Method::POST,
            "/tenants",
            Some(new_shop(&format!("shop-{i}"))),
            &h,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "origin={origin:?}");
    }
    assert_eq!(shops().await, 0);

    // 白名單內的 Origin 才可以
    let (status, _, body) = raw(
        &app,
        Method::POST,
        "/tenants",
        Some(new_shop("good")),
        &[(header::COOKIE, &cookie), (header::ORIGIN, ORIGIN)],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(shops().await, 1);

    // 其他會改資料的方法也一樣(換成真的存在的路由,否則會先得到 404 / 405)
    let id = uuid::Uuid::new_v4();
    for (method, uri) in [
        (Method::PATCH, "/t/good".to_string()),
        (Method::PUT, format!("/t/good/members/{id}/working-hours")),
        (Method::DELETE, format!("/t/good/services/{id}")),
    ] {
        let (status, _, _) = raw(
            &app,
            method.clone(),
            &uri,
            Some(json!({"name": "改名", "hours": []})),
            &[
                (header::COOKIE, &cookie),
                (header::ORIGIN, "http://evil.test"),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
}

#[sqlx::test]
async fn bearer_needs_no_origin_and_takes_precedence_over_the_cookie(pool: PgPool) {
    let (app, cookie) = setup(pool).await;
    let (_, body) = login(&app, "a@example.com").await;
    let token = body["token"].as_str().unwrap();

    // Bearer 不是瀏覽器自動帶的,不檢查 Origin
    let (status, _, _) = raw(
        &app,
        Method::POST,
        "/tenants",
        Some(new_shop("by-bearer")),
        &[(header::AUTHORIZATION, &format!("Bearer {token}"))],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // 帶了無效的 Bearer 就是 401,不會退回去用有效的 cookie
    let (status, _, _) = raw(
        &app,
        Method::GET,
        "/auth/me",
        None,
        &[
            (header::AUTHORIZATION, "Bearer garbage"),
            (header::COOKIE, &cookie),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn bad_cookies_are_rejected(pool: PgPool) {
    let (app, cookie) = setup(pool).await;
    let jwt = cookie.strip_prefix("session=").unwrap();
    for bad in [
        "session=garbage".to_string(),
        "session=".to_string(),
        format!("xsession={jwt}"), // 名稱必須完全相符
        format!("SESSION={jwt}"),
        "theme=dark".to_string(),
    ] {
        let (status, _, _) = raw(
            &app,
            Method::GET,
            "/auth/me",
            None,
            &[(header::COOKIE, &bad)],
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{bad}");
    }
    // 沒有任何憑證
    assert_eq!(
        call(&app, Method::GET, "/auth/me", None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test]
async fn password_change_revokes_cookie_sessions_too(pool: PgPool) {
    let (app, cookie) = setup(pool.clone()).await;
    assert_eq!(
        raw(
            &app,
            Method::GET,
            "/auth/me",
            None,
            &[(header::COOKIE, &cookie)]
        )
        .await
        .0,
        StatusCode::OK
    );
    sqlx::query("UPDATE users SET password_changed_at = now() + interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        raw(
            &app,
            Method::GET,
            "/auth/me",
            None,
            &[(header::COOKIE, &cookie)]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test]
async fn logout_clears_the_cookie_even_when_not_logged_in(pool: PgPool) {
    let (app, cookie) = setup(pool).await;
    for headers in [vec![(header::COOKIE, cookie.as_str())], vec![]] {
        let (status, h, _) = raw(&app, Method::POST, "/auth/logout", None, &headers).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let cleared = set_cookie(&h);
        assert!(cleared.starts_with("session=;"), "{cleared}");
        assert!(
            cleared.contains("Max-Age=0") && cleared.contains("HttpOnly"),
            "{cleared}"
        );
    }
}
