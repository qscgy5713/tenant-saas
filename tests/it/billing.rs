use crate::common;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::common::{add_member, call, create_tenant, signup, test_config};
use axum::{
    Form, Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Method, Request, StatusCode, header},
    routing::post,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tenant_saas::{
    billing::{self, SignatureError},
    config::{Config, StripeConfig, parse_stripe},
    mail::{Mailer, MemoryMailer},
    routes::{self, AppState},
    worker::tick,
};
use tower::ServiceExt;

const WEBHOOK_SECRET: &str = "whsec_test_secret";

// ---------- 簽章 ----------

#[test]
fn signature_accepts_only_the_exact_payload_signed_recently() {
    let body = br#"{"id":"evt_1"}"#;
    let now = 1_800_000_000;
    let header = billing::sign(WEBHOOK_SECRET, now, body);
    assert_eq!(
        billing::verify_signature(WEBHOOK_SECRET, &header, body, now),
        Ok(())
    );
    // 容忍度內(前後 5 分鐘)
    assert_eq!(
        billing::verify_signature(WEBHOOK_SECRET, &header, body, now + 299),
        Ok(())
    );
    assert_eq!(
        billing::verify_signature(WEBHOOK_SECRET, &header, body, now - 299),
        Ok(())
    );
    // body 被改、金鑰不對、太舊 / 太未來、格式不對
    assert_eq!(
        billing::verify_signature(WEBHOOK_SECRET, &header, br#"{"id":"evt_2"}"#, now),
        Err(SignatureError::Mismatch)
    );
    assert_eq!(
        billing::verify_signature("whsec_other", &header, body, now),
        Err(SignatureError::Mismatch)
    );
    assert_eq!(
        billing::verify_signature(WEBHOOK_SECRET, &header, body, now + 301),
        Err(SignatureError::Expired)
    );
    assert_eq!(
        billing::verify_signature(WEBHOOK_SECRET, &header, body, now - 301),
        Err(SignatureError::Expired)
    );
    for bad in [
        "",
        "garbage",
        "t=123",
        "v1=abcd",
        "t=abc,v1=00",
        "t=1800000000,v1=zz",
        "t=1800000000,v1=abc",
    ] {
        assert!(
            billing::verify_signature(WEBHOOK_SECRET, bad, body, now).is_err(),
            "{bad}"
        );
    }
    // 時間戳被竄改成「現在」:簽章對不上(簽章涵蓋時間戳)
    let sig = header.split("v1=").nth(1).unwrap();
    let forged = format!("t={},v1={sig}", now + 100);
    assert_eq!(
        billing::verify_signature(WEBHOOK_SECRET, &forged, body, now + 100),
        Err(SignatureError::Mismatch)
    );
}

#[test]
fn signature_with_several_v1_values_matches_any() {
    // Stripe 輪替 webhook 密鑰期間,會同時附上新舊兩個簽章
    let body = b"{}";
    let now = 1_800_000_000;
    let good = billing::sign(WEBHOOK_SECRET, now, body);
    let good_sig = good.split("v1=").nth(1).unwrap();
    let header = format!("t={now},v1={},v1={good_sig}", "00".repeat(32));
    assert_eq!(
        billing::verify_signature(WEBHOOK_SECRET, &header, body, now),
        Ok(())
    );
}

// ---------- 假的 Stripe ----------

type Call = (String, HashMap<String, String>, HeaderMap);

#[derive(Clone, Default)]
struct FakeStripe {
    calls: Arc<Mutex<Vec<Call>>>,
    fail: Arc<Mutex<bool>>,
}

impl FakeStripe {
    fn calls_to(&self, path: &str) -> Vec<(HashMap<String, String>, HeaderMap)> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.0 == path)
            .map(|c| (c.1.clone(), c.2.clone()))
            .collect()
    }
}

async fn start_fake_stripe() -> (String, FakeStripe) {
    let fake = FakeStripe::default();
    async fn handle(
        State(f): State<FakeStripe>,
        uri: axum::http::Uri,
        headers: HeaderMap,
        Form(form): Form<HashMap<String, String>>,
    ) -> (StatusCode, Json<Value>) {
        f.calls
            .lock()
            .unwrap()
            .push((uri.path().to_string(), form, headers));
        if *f.fail.lock().unwrap() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": {"message": "internal secret detail"}})),
            );
        }
        let body = match uri.path() {
            "/v1/customers" => json!({"id": "cus_123"}),
            "/v1/checkout/sessions" => json!({"url": "https://checkout.stripe.test/session/abc"}),
            _ => json!({"url": "https://billing.stripe.test/portal/xyz"}),
        };
        (StatusCode::OK, Json(body))
    }
    let router = Router::new()
        .route("/v1/customers", post(handle))
        .route("/v1/checkout/sessions", post(handle))
        .route("/v1/billing_portal/sessions", post(handle))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (base, fake)
}

fn stripe_app(pool: PgPool, api_base: &str) -> Router {
    let config = Config {
        stripe: Some(StripeConfig {
            secret_key: "sk_test_fake".into(),
            webhook_secret: WEBHOOK_SECRET.into(),
            api_base: api_base.into(),
            prices: vec![
                ("pro".into(), "price_pro".into()),
                ("business".into(), "price_biz".into()),
            ],
        }),
        ..test_config(3600)
    };
    routes::router(AppState::new(pool, &config))
}

async fn webhook_raw(app: &Router, body: &[u8], signature: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/webhooks/stripe")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(s) = signature {
        req = req.header("stripe-signature", s);
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::from(body.to_vec())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// 以正確的簽章送出事件
async fn send_event(app: &Router, event: Value) -> (StatusCode, Value) {
    let body = event.to_string();
    let sig = billing::sign(
        WEBHOOK_SECRET,
        chrono::Utc::now().timestamp(),
        body.as_bytes(),
    );
    webhook_raw(app, body.as_bytes(), Some(&sig)).await
}

fn sub_event(
    id: &str,
    kind: &str,
    created: i64,
    customer: &str,
    status: &str,
    price: &str,
) -> Value {
    json!({
        "id": id, "type": kind, "created": created,
        "data": {"object": {
            "id": "sub_1", "customer": customer, "status": status, "cancel_at_period_end": false,
            "items": {"data": [{"price": {"id": price}, "current_period_end": 1_900_000_000}]}
        }}
    })
}

struct Shop {
    app: Router,
    owner: String,
    pool: PgPool,
    fake: FakeStripe,
}

async fn shop(pool: PgPool) -> Shop {
    let (base, fake) = start_fake_stripe().await;
    let app = stripe_app(pool.clone(), &base);
    let owner = signup(&app, "owner@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    Shop {
        app,
        owner,
        pool,
        fake,
    }
}

async fn plan_of(pool: &PgPool) -> String {
    sqlx::query_scalar("SELECT plan_id FROM tenants WHERE slug = 'shop-a'")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// 讓店家走完「綁定客戶」這一步,回傳客戶編號
async fn start_checkout(s: &Shop, plan: &str) -> (StatusCode, Value) {
    call(
        &s.app,
        Method::POST,
        "/t/shop-a/billing/checkout",
        Some(json!({"plan": plan})),
        Some(&s.owner),
    )
    .await
}

// ---------- 未啟用 ----------

#[sqlx::test]
async fn billing_is_disabled_without_stripe_config(pool: PgPool) {
    let app = common::app(pool);
    let owner = signup(&app, "o@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    let (status, body) = call(&app, Method::GET, "/t/shop-a/billing", None, Some(&owner)).await;
    assert_eq!((status, &body["enabled"]), (StatusCode::OK, &json!(false)));
    for (m, uri) in [
        (Method::POST, "/t/shop-a/billing/checkout"),
        (Method::POST, "/t/shop-a/billing/portal"),
    ] {
        let (status, _) = call(&app, m, uri, Some(json!({"plan": "pro"})), Some(&owner)).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }
    assert_eq!(
        webhook_raw(&app, b"{}", Some("t=1,v1=00")).await.0,
        StatusCode::NOT_IMPLEMENTED
    );
}

#[test]
fn stripe_config_requires_both_secrets_and_a_price() {
    // 只設一個、或沒有任何 price,啟動就該失敗,而不是上線後 webhook 默默收不到
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |k: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.to_string())
        }
    };
    assert!(
        parse_stripe(env(&[])).unwrap().is_none(),
        "完全沒設定 = 不啟用"
    );
    assert!(
        parse_stripe(env(&[("STRIPE_SECRET_KEY", "sk")])).is_err(),
        "只設 secret key"
    );
    assert!(
        parse_stripe(env(&[("STRIPE_WEBHOOK_SECRET", "wh")])).is_err(),
        "只設 webhook secret"
    );
    assert!(
        parse_stripe(env(&[
            ("STRIPE_SECRET_KEY", "sk"),
            ("STRIPE_WEBHOOK_SECRET", "wh")
        ]))
        .is_err(),
        "沒有任何 price"
    );
    assert!(
        parse_stripe(env(&[
            ("STRIPE_SECRET_KEY", ""),
            ("STRIPE_WEBHOOK_SECRET", "")
        ]))
        .unwrap()
        .is_none(),
        "空字串視為未設定"
    );

    let cfg = parse_stripe(env(&[
        ("STRIPE_SECRET_KEY", "sk"),
        ("STRIPE_WEBHOOK_SECRET", "wh"),
        ("STRIPE_PRICE_PRO", "price_1"),
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(cfg.prices, vec![("pro".to_string(), "price_1".to_string())]);
    assert_eq!(cfg.api_base, "https://api.stripe.com");
}

// ---------- 結帳 ----------

#[sqlx::test]
async fn only_the_owner_can_start_checkout(pool: PgPool) {
    let s = shop(pool.clone()).await;
    for (email, role) in [("m@example.com", "manager"), ("st@example.com", "staff")] {
        let token = signup(&s.app, email).await;
        add_member(&pool, "shop-a", email, role).await;
        for (m, uri) in [
            (Method::GET, "/t/shop-a/billing"),
            (Method::POST, "/t/shop-a/billing/checkout"),
            (Method::POST, "/t/shop-a/billing/portal"),
        ] {
            let (status, _) =
                call(&s.app, m, uri, Some(json!({"plan": "pro"})), Some(&token)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{role} {uri}");
        }
    }
    assert!(
        s.fake.calls.lock().unwrap().is_empty(),
        "沒有權限的請求不該打到 Stripe"
    );
}

#[sqlx::test]
async fn checkout_creates_one_customer_and_a_session_for_the_chosen_plan(pool: PgPool) {
    let s = shop(pool).await;
    let (status, body) = start_checkout(&s, "pro").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["url"], "https://checkout.stripe.test/session/abc");

    let customers = s.fake.calls_to("/v1/customers");
    assert_eq!(customers.len(), 1);
    assert_eq!(customers[0].0["metadata[tenant_id]"].len(), 36);
    assert!(
        customers[0].1["idempotency-key"]
            .to_str()
            .unwrap()
            .starts_with("tenant-customer-")
    );
    assert_eq!(
        customers[0].1[header::AUTHORIZATION].to_str().unwrap(),
        "Bearer sk_test_fake"
    );

    let sessions = s.fake.calls_to("/v1/checkout/sessions");
    let form = &sessions[0].0;
    assert_eq!(form["mode"], "subscription");
    assert_eq!(form["customer"], "cus_123");
    assert_eq!(form["line_items[0][price]"], "price_pro");
    assert_eq!(
        form["success_url"],
        "http://app.test/admin/shop-a/plan?checkout=success"
    );
    assert_eq!(
        form["cancel_url"],
        "http://app.test/admin/shop-a/plan?checkout=cancel"
    );

    // 再結帳一次(例如取消後重來):重用同一個客戶,不再建立
    start_checkout(&s, "business").await;
    assert_eq!(s.fake.calls_to("/v1/customers").len(), 1);
    assert_eq!(
        s.fake.calls_to("/v1/checkout/sessions")[1].0["line_items[0][price]"],
        "price_biz"
    );

    // 方案不會因為「開始結帳」就改變 —— 只有 webhook 能改
    assert_eq!(plan_of(&s.pool).await, "free");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action = 'billing.checkout_started'",
    )
    .fetch_one(&s.pool)
    .await
    .unwrap();
    assert_eq!(n, 2);
}

#[sqlx::test]
async fn checkout_rejects_unknown_plans_and_hides_stripe_failures(pool: PgPool) {
    let s = shop(pool).await;
    for plan in ["free", "enterprise", "", "PRO"] {
        assert_eq!(
            start_checkout(&s, plan).await.0,
            StatusCode::BAD_REQUEST,
            "{plan}"
        );
    }
    assert!(s.fake.calls.lock().unwrap().is_empty());

    *s.fake.fail.lock().unwrap() = true;
    let (status, body) = start_checkout(&s, "pro").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(
        !body.to_string().contains("secret detail"),
        "Stripe 的錯誤細節不可回給使用者"
    );
}

// ---------- webhook ----------

#[sqlx::test]
async fn webhook_requires_a_valid_signature(pool: PgPool) {
    let s = shop(pool).await;
    start_checkout(&s, "pro").await;
    let body = sub_event(
        "evt_1",
        "customer.subscription.created",
        100,
        "cus_123",
        "active",
        "price_pro",
    )
    .to_string();
    let now = chrono::Utc::now().timestamp();

    assert_eq!(
        webhook_raw(&s.app, body.as_bytes(), None).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        webhook_raw(&s.app, body.as_bytes(), Some("t=1,v1=00"))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let wrong_key = billing::sign("whsec_other", now, body.as_bytes());
    assert_eq!(
        webhook_raw(&s.app, body.as_bytes(), Some(&wrong_key))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let old = billing::sign(WEBHOOK_SECRET, now - 3600, body.as_bytes());
    assert_eq!(
        webhook_raw(&s.app, body.as_bytes(), Some(&old)).await.0,
        StatusCode::BAD_REQUEST,
        "重放舊簽章"
    );
    let signed_other = billing::sign(WEBHOOK_SECRET, now, b"{}");
    assert_eq!(
        webhook_raw(&s.app, body.as_bytes(), Some(&signed_other))
            .await
            .0,
        StatusCode::BAD_REQUEST,
        "簽章與內容不符"
    );
    assert_eq!(plan_of(&s.pool).await, "free", "驗證失敗的事件不能改方案");

    // 簽章正確但不是有效 JSON
    let sig = billing::sign(WEBHOOK_SECRET, now, b"not json");
    assert_eq!(
        webhook_raw(&s.app, b"not json", Some(&sig)).await.0,
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test]
async fn subscription_lifecycle_changes_the_plan(pool: PgPool) {
    let s = shop(pool).await;
    start_checkout(&s, "pro").await;

    // 付款完成 → 升級
    let (status, body) = send_event(
        &s.app,
        sub_event(
            "evt_1",
            "customer.subscription.created",
            100,
            "cus_123",
            "active",
            "price_pro",
        ),
    )
    .await;
    assert_eq!(
        (status, body["result"].as_str()),
        (StatusCode::OK, Some("applied"))
    );
    assert_eq!(plan_of(&s.pool).await, "pro");

    // 稽核紀錄(system 操作)
    let detail: Value =
        sqlx::query_scalar("SELECT detail FROM audit_logs WHERE action = 'billing.plan_changed'")
            .fetch_one(&s.pool)
            .await
            .unwrap();
    assert_eq!(
        (detail["from"].as_str(), detail["to"].as_str()),
        (Some("free"), Some("pro"))
    );

    // 狀態與到期日顯示給店主
    let (_, overview) = call(
        &s.app,
        Method::GET,
        "/t/shop-a/billing",
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(overview["subscription"]["status"], "active");
    assert!(overview["subscription"]["current_period_end"].is_string());
    assert_eq!(
        (
            overview["can_checkout"].clone(),
            overview["can_manage"].clone()
        ),
        (json!(false), json!(true))
    );

    // 升級到企業版
    send_event(
        &s.app,
        sub_event(
            "evt_2",
            "customer.subscription.updated",
            200,
            "cus_123",
            "active",
            "price_biz",
        ),
    )
    .await;
    assert_eq!(plan_of(&s.pool).await, "business");

    // 付款失敗進入 past_due:寬限期,仍保有付費方案
    send_event(
        &s.app,
        sub_event(
            "evt_3",
            "customer.subscription.updated",
            300,
            "cus_123",
            "past_due",
            "price_biz",
        ),
    )
    .await;
    assert_eq!(plan_of(&s.pool).await, "business");

    // 訂閱結束 → 退回免費版,而且可以重新訂閱
    send_event(
        &s.app,
        sub_event(
            "evt_4",
            "customer.subscription.deleted",
            400,
            "cus_123",
            "canceled",
            "price_biz",
        ),
    )
    .await;
    assert_eq!(plan_of(&s.pool).await, "free");
    let (_, overview) = call(
        &s.app,
        Method::GET,
        "/t/shop-a/billing",
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(overview["can_checkout"], json!(true));
}

#[sqlx::test]
async fn duplicate_and_out_of_order_events_do_not_change_the_result(pool: PgPool) {
    let s = shop(pool).await;
    start_checkout(&s, "pro").await;
    let created = sub_event(
        "evt_1",
        "customer.subscription.created",
        100,
        "cus_123",
        "active",
        "price_pro",
    );
    assert_eq!(
        send_event(&s.app, created.clone()).await.1["result"],
        "applied"
    );
    assert_eq!(
        send_event(&s.app, created).await.1["result"],
        "duplicate",
        "Stripe 重送同一個事件"
    );

    // 取消事件先到(created=300),較早的更新事件(created=200)晚到 → 不可讓方案復活
    send_event(
        &s.app,
        sub_event(
            "evt_3",
            "customer.subscription.deleted",
            300,
            "cus_123",
            "canceled",
            "price_pro",
        ),
    )
    .await;
    assert_eq!(plan_of(&s.pool).await, "free");
    let late = send_event(
        &s.app,
        sub_event(
            "evt_2",
            "customer.subscription.updated",
            200,
            "cus_123",
            "active",
            "price_pro",
        ),
    )
    .await;
    assert_eq!(late.1["result"], "stale");
    assert_eq!(plan_of(&s.pool).await, "free");
}

#[sqlx::test]
async fn unrelated_unknown_and_misconfigured_events_are_handled_safely(pool: PgPool) {
    let s = shop(pool).await;
    start_checkout(&s, "pro").await;

    // 與方案無關的事件:收下(200),不然 Stripe 會一直重送
    let (status, body) = send_event(
        &s.app,
        json!({"id": "evt_x", "type": "charge.succeeded", "created": 1, "data": {"object": {}}}),
    )
    .await;
    assert_eq!(
        (status, body["result"].as_str()),
        (StatusCode::OK, Some("ignored"))
    );

    // invoice.paid 缺客戶 / 發票編號(沒有客戶的一次性發票):也是收下,不能回 400 讓 Stripe 一直重送
    let (status, body) = send_event(
        &s.app,
        json!({"id": "evt_nocust", "type": "invoice.paid", "created": 1, "data": {"object": {"id": "in_x"}}}),
    )
    .await;
    assert_eq!(
        (status, body["result"].as_str()),
        (StatusCode::OK, Some("ignored")),
        "{body}"
    );

    // 不認識的客戶(不是我們的):不改任何東西
    let (status, body) = send_event(
        &s.app,
        sub_event(
            "evt_y",
            "customer.subscription.created",
            1,
            "cus_stranger",
            "active",
            "price_pro",
        ),
    )
    .await;
    assert_eq!(
        (status, body["result"].as_str()),
        (StatusCode::OK, Some("unknown_customer"))
    );
    assert_eq!(plan_of(&s.pool).await, "free");

    // 付費中卻對不到方案(price 沒設定):回 500 讓 Stripe 重送,而且這個事件不能被記成「已處理」
    let (status, _) = send_event(
        &s.app,
        sub_event(
            "evt_z",
            "customer.subscription.created",
            5,
            "cus_123",
            "active",
            "price_mystery",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let recorded: i64 = sqlx::query_scalar("SELECT count(*) FROM stripe_events WHERE id = 'evt_z'")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(recorded, 0);
    assert_eq!(plan_of(&s.pool).await, "free");

    // 尚未完成付款(incomplete)不升級
    send_event(
        &s.app,
        sub_event(
            "evt_i",
            "customer.subscription.created",
            6,
            "cus_123",
            "incomplete",
            "price_pro",
        ),
    )
    .await;
    assert_eq!(plan_of(&s.pool).await, "free");
}

#[sqlx::test]
async fn an_active_subscription_cannot_be_checked_out_again_but_can_use_the_portal(pool: PgPool) {
    let s = shop(pool).await;
    // 沒訂閱過 → 沒有入口可進
    assert_eq!(
        call(
            &s.app,
            Method::POST,
            "/t/shop-a/billing/portal",
            None,
            Some(&s.owner)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );

    start_checkout(&s, "pro").await;
    send_event(
        &s.app,
        sub_event(
            "evt_1",
            "customer.subscription.created",
            100,
            "cus_123",
            "active",
            "price_pro",
        ),
    )
    .await;

    let (status, body) = start_checkout(&s, "business").await;
    assert_eq!(status, StatusCode::CONFLICT, "避免重複訂閱重複扣款: {body}");

    let (status, body) = call(
        &s.app,
        Method::POST,
        "/t/shop-a/billing/portal",
        None,
        Some(&s.owner),
    )
    .await;
    assert_eq!(
        (status, body["url"].as_str()),
        (
            StatusCode::OK,
            Some("https://billing.stripe.test/portal/xyz")
        )
    );
    let portal = &s.fake.calls_to("/v1/billing_portal/sessions")[0].0;
    assert_eq!(
        (portal["customer"].as_str(), portal["return_url"].as_str()),
        ("cus_123", "http://app.test/admin/shop-a/plan")
    );
}

#[sqlx::test]
async fn tenants_and_their_members_cannot_touch_billing_tables(pool: PgPool) {
    let s = shop(pool.clone()).await;
    start_checkout(&s, "pro").await;
    let tenant: uuid::Uuid = sqlx::query_scalar("SELECT id FROM tenants")
        .fetch_one(&pool)
        .await
        .unwrap();
    let uid = common::user_id(&pool, "owner@example.com").await;

    // 租戶角色不能直接讀 / 改計費資料,也不能自己改方案
    for sql in [
        "SELECT * FROM tenant_billing",
        "UPDATE tenant_billing SET status = 'active'",
        "SELECT * FROM stripe_events",
        "UPDATE tenants SET plan_id = 'business'",
    ] {
        let mut tx = tenant_saas::db::begin_scoped(&pool, Some(uid), Some(tenant))
            .await
            .unwrap();
        let err = sqlx::query(sql).execute(&mut *tx).await.unwrap_err();
        assert_eq!(
            err.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501"),
            "{sql}"
        );
    }
    // 租戶上下文內也不能呼叫 webhook 專用的函式
    let mut tx = tenant_saas::db::begin_scoped(&pool, Some(uid), Some(tenant))
        .await
        .unwrap();
    let err = sqlx::query(
        "SELECT billing_apply_event('e','t',1,'cus_123','s','active','business',now(),false)",
    )
    .execute(&mut *tx)
    .await
    .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("42501")
    );
}

// ---------- 付款失敗通知 ----------

fn payment_failed_event(id: &str, customer: &str, attempt: i64, next: Option<i64>) -> Value {
    json!({
        "id": id, "type": "invoice.payment_failed", "created": 1_700_000_000,
        "data": {"object": {
            "id": "in_1", "customer": customer, "amount_due": 150_000, "currency": "twd",
            "attempt_count": attempt, "next_payment_attempt": next
        }}
    })
}

/// 寄出佇列裡的信,回傳 (收件者, 標題, 內容)
async fn deliver(pool: &PgPool) -> Vec<(String, String, String)> {
    let memory = MemoryMailer::new();
    tick(pool, &Mailer::Memory(memory.clone()), "http://app.test")
        .await
        .unwrap();
    memory
        .sent()
        .into_iter()
        .map(|m| (m.to, m.subject, m.body))
        .collect()
}

async fn payment_failed_audits(pool: &PgPool) -> Vec<Value> {
    sqlx::query_scalar("SELECT detail FROM audit_logs WHERE action = 'billing.payment_failed'")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[test]
fn amounts_are_formatted_by_currency() {
    assert_eq!(billing::format_amount(150_000, "twd"), "TWD 1,500.00");
    assert_eq!(billing::format_amount(5, "usd"), "USD 0.05");
    assert_eq!(billing::format_amount(0, "usd"), "USD 0.00");
    assert_eq!(
        billing::format_amount(123_456_789, "usd"),
        "USD 1,234,567.89"
    );
    assert_eq!(billing::format_amount(100_000, "USD"), "USD 1,000.00");
    // 沒有小數位的幣別不能除以 100
    assert_eq!(billing::format_amount(1500, "jpy"), "JPY 1,500");
    assert_eq!(billing::format_amount(999, "jpy"), "JPY 999");
    assert_eq!(billing::format_amount(-250, "usd"), "USD -2.50");
}

#[sqlx::test]
async fn failed_payment_notifies_only_the_owners_and_leaves_the_plan_alone(pool: PgPool) {
    let s = shop(pool).await;
    start_checkout(&s, "pro").await;
    send_event(
        &s.app,
        sub_event(
            "evt_sub",
            "customer.subscription.created",
            100,
            "cus_123",
            "active",
            "price_pro",
        ),
    )
    .await;
    assert_eq!(plan_of(&s.pool).await, "pro");
    // 另一位店主與一位員工:店主都要收到,員工不用
    signup(&s.app, "owner2@example.com").await;
    add_member(&s.pool, "shop-a", "owner2@example.com", "owner").await;
    signup(&s.app, "staff@example.com").await;
    add_member(&s.pool, "shop-a", "staff@example.com", "staff").await;
    deliver(&s.pool).await; // 清掉先前的信

    let (status, body) = send_event(
        &s.app,
        payment_failed_event("evt_fail_1", "cus_123", 2, Some(1_800_000_000)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"], "applied");

    let mut sent = deliver(&s.pool).await;
    sent.sort();
    let recipients: Vec<_> = sent.iter().map(|m| m.0.as_str()).collect();
    assert_eq!(recipients, ["owner2@example.com", "owner@example.com"]);
    let (_, subject, text) = &sent[0];
    assert!(subject.contains("店 shop-a"), "{subject}");
    assert!(text.contains("TWD 1,500.00"), "{text}");
    assert!(text.contains("第 2 次"), "{text}");
    assert!(text.contains("2027-01-15"), "{text}");
    assert!(text.contains("http://app.test/admin/shop-a/plan"), "{text}");
    assert!(!text.contains('{'), "樣板的代入欄位沒有被取代: {text}");

    // 通知本身不改方案(方案跟著訂閱事件走)
    assert_eq!(plan_of(&s.pool).await, "pro");
    assert_eq!(
        payment_failed_audits(&s.pool).await,
        [json!({"attempt": 2, "final": false})]
    );
}

#[sqlx::test]
async fn redelivered_failure_event_sends_one_mail_and_one_audit(pool: PgPool) {
    let s = shop(pool).await;
    start_checkout(&s, "pro").await;
    deliver(&s.pool).await;

    let ev = payment_failed_event("evt_fail_1", "cus_123", 1, Some(1_800_000_000));
    assert_eq!(send_event(&s.app, ev.clone()).await.1["result"], "applied");
    assert_eq!(send_event(&s.app, ev).await.1["result"], "duplicate");

    assert_eq!(deliver(&s.pool).await.len(), 1);
    assert_eq!(payment_failed_audits(&s.pool).await.len(), 1);
    // 同一張發票的下一次嘗試是不同的事件,要再通知一次
    send_event(
        &s.app,
        payment_failed_event("evt_fail_2", "cus_123", 2, Some(1_800_000_000)),
    )
    .await;
    assert_eq!(deliver(&s.pool).await.len(), 1);
}

#[sqlx::test]
async fn last_attempt_says_so_and_has_no_retry_date(pool: PgPool) {
    let s = shop(pool).await;
    start_checkout(&s, "pro").await;
    deliver(&s.pool).await;

    send_event(
        &s.app,
        payment_failed_event("evt_final", "cus_123", 4, None),
    )
    .await;
    let sent = deliver(&s.pool).await;
    assert_eq!(sent.len(), 1);
    let text = &sent[0].2;
    assert!(text.contains("最後一次"), "{text}");
    assert!(!text.contains("再試一次"), "{text}");
    assert!(!text.contains('{'), "{text}");
    assert_eq!(
        payment_failed_audits(&s.pool).await,
        [json!({"attempt": 4, "final": true})]
    );
}

#[sqlx::test]
async fn failure_for_a_stranger_or_a_malformed_event_sends_nothing(pool: PgPool) {
    let s = shop(pool).await;
    start_checkout(&s, "pro").await;
    deliver(&s.pool).await;

    // 不是我們的客戶:收下(200),不寄信、不寫稽核
    let (status, body) = send_event(
        &s.app,
        payment_failed_event("evt_s", "cus_stranger", 1, None),
    )
    .await;
    assert_eq!(
        (status, body["result"].as_str()),
        (StatusCode::OK, Some("unknown_customer"))
    );
    assert!(deliver(&s.pool).await.is_empty());
    assert!(payment_failed_audits(&s.pool).await.is_empty());

    // 缺少必要欄位:400,而且事件沒有被記成「已處理」
    let (status, _) = send_event(
        &s.app,
        json!({"id": "evt_bad", "type": "invoice.payment_failed", "created": 1,
               "data": {"object": {"id": "in_1", "amount_due": 1, "currency": "twd"}}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM stripe_events WHERE id = 'evt_bad'")
            .fetch_one(&s.pool)
            .await
            .unwrap();
    assert_eq!(recorded, 0);
}

#[sqlx::test]
async fn failure_notice_needs_a_valid_signature(pool: PgPool) {
    let s = shop(pool).await;
    start_checkout(&s, "pro").await;
    deliver(&s.pool).await;

    let body = payment_failed_event("evt_unsigned", "cus_123", 1, None).to_string();
    let (status, _) = webhook_raw(&s.app, body.as_bytes(), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = webhook_raw(&s.app, body.as_bytes(), Some("t=1,v1=00")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(deliver(&s.pool).await.is_empty());
}

// ---------- 付款恢復通知 ----------

fn invoice_paid_event(id: &str, customer: &str, invoice: &str) -> Value {
    json!({
        "id": id, "type": "invoice.paid", "created": 1_700_000_100,
        "data": {"object": {"id": invoice, "customer": customer, "amount_paid": 150_000, "currency": "twd"}}
    })
}

fn payment_failed_for(id: &str, customer: &str, invoice: &str) -> Value {
    json!({
        "id": id, "type": "invoice.payment_failed", "created": 1_700_000_000,
        "data": {"object": {
            "id": invoice, "customer": customer, "amount_due": 150_000, "currency": "twd",
            "attempt_count": 1, "next_payment_attempt": 1_800_000_000
        }}
    })
}

async fn recovered_audits(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action = 'billing.payment_recovered'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn paying_the_failed_invoice_tells_the_owners_it_recovered(pool: PgPool) {
    let s = shop(pool).await;
    start_checkout(&s, "pro").await;
    signup(&s.app, "owner2@example.com").await;
    add_member(&s.pool, "shop-a", "owner2@example.com", "owner").await;
    signup(&s.app, "staff@example.com").await;
    add_member(&s.pool, "shop-a", "staff@example.com", "staff").await;
    deliver(&s.pool).await;

    send_event(&s.app, payment_failed_for("evt_f", "cus_123", "in_1")).await;
    deliver(&s.pool).await; // 失敗通知先寄掉
    let (status, body) = send_event(&s.app, invoice_paid_event("evt_p", "cus_123", "in_1")).await;
    assert_eq!(
        (status, body["result"].as_str()),
        (StatusCode::OK, Some("applied"))
    );

    let mut sent = deliver(&s.pool).await;
    sent.sort();
    let to: Vec<_> = sent.iter().map(|m| m.0.as_str()).collect();
    assert_eq!(to, ["owner2@example.com", "owner@example.com"], "只有店主");
    assert!(
        sent[0].1.contains("付款已恢復") && sent[0].1.contains("店 shop-a"),
        "{}",
        sent[0].1
    );
    assert!(
        sent[0].2.contains("http://app.test/admin/shop-a/plan"),
        "{}",
        sent[0].2
    );
    assert!(
        !sent[0].2.contains('{'),
        "樣板欄位都要被代入: {}",
        sent[0].2
    );
    assert_eq!(recovered_audits(&s.pool).await, 1);

    // 重送同一個事件:不會再寄、不會再記
    let (_, again) = send_event(&s.app, invoice_paid_event("evt_p", "cus_123", "in_1")).await;
    assert_eq!(again["result"], "duplicate");
    assert!(deliver(&s.pool).await.is_empty());
    // 同一張發票再來一個不同的付清事件(Stripe 偶爾會對同一張發票發多個事件):已經恢復過,不再通知
    let (_, second) = send_event(&s.app, invoice_paid_event("evt_p2", "cus_123", "in_1")).await;
    assert_eq!(second["result"], "no_prior_failure");
    assert!(deliver(&s.pool).await.is_empty());
    assert_eq!(recovered_audits(&s.pool).await, 1);
}

#[sqlx::test]
async fn normal_renewals_and_other_invoices_send_no_recovery_mail(pool: PgPool) {
    let s = shop(pool).await;
    start_checkout(&s, "pro").await;
    deliver(&s.pool).await;

    // 沒失敗過的發票付清 = 正常續訂,不寄信
    let (_, body) = send_event(&s.app, invoice_paid_event("evt_p1", "cus_123", "in_ok")).await;
    assert_eq!(body["result"], "no_prior_failure");
    assert!(deliver(&s.pool).await.is_empty());

    // in_1 失敗;之後付清的是另一張發票(in_2)→ in_1 仍然沒恢復,不能說「恢復了」
    send_event(&s.app, payment_failed_for("evt_f", "cus_123", "in_1")).await;
    deliver(&s.pool).await;
    let (_, body) = send_event(&s.app, invoice_paid_event("evt_p2", "cus_123", "in_2")).await;
    assert_eq!(body["result"], "no_prior_failure");
    assert!(deliver(&s.pool).await.is_empty());
    assert_eq!(recovered_audits(&s.pool).await, 0);
    // in_1 真的付清了 → 這時才通知
    let (_, body) = send_event(&s.app, invoice_paid_event("evt_p3", "cus_123", "in_1")).await;
    assert_eq!(body["result"], "applied");
    assert_eq!(deliver(&s.pool).await.len(), 1);

    // 不是我們的客戶:收下(200),什麼都不寄
    let (status, body) =
        send_event(&s.app, invoice_paid_event("evt_p4", "cus_stranger", "in_9")).await;
    assert_eq!(
        (status, body["result"].as_str()),
        (StatusCode::OK, Some("unknown_customer"))
    );
    // 缺少必要欄位(沒有客戶):忽略但仍回 200(不讓 Stripe 重送),事件沒被記成「已處理」
    let (status, _) = send_event(
        &s.app,
        json!({"id": "evt_bad", "type": "invoice.paid", "created": 1, "data": {"object": {"id": "in_1"}}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let recorded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM stripe_events WHERE id = 'evt_bad'")
            .fetch_one(&s.pool)
            .await
            .unwrap();
    assert_eq!(recorded, 0);
}

/// Stripe 不保證順序:付清的事件比失敗的事件先到 → 不能再寄一封已經過時的「扣款失敗」
#[sqlx::test]
async fn a_failure_event_arriving_after_the_invoice_was_paid_sends_nothing(pool: PgPool) {
    let s = shop(pool).await;
    start_checkout(&s, "pro").await;
    deliver(&s.pool).await;

    send_event(&s.app, invoice_paid_event("evt_p", "cus_123", "in_1")).await;
    let (_, late) = send_event(&s.app, payment_failed_for("evt_f", "cus_123", "in_1")).await;
    assert_eq!(late["result"], "already_paid");
    assert!(
        deliver(&s.pool).await.is_empty(),
        "已經付清的發票不該再收到失敗通知"
    );
    assert!(payment_failed_audits(&s.pool).await.is_empty());

    // 別張發票的失敗仍然照常通知
    let (_, other) = send_event(&s.app, payment_failed_for("evt_f2", "cus_123", "in_2")).await;
    assert_eq!(other["result"], "applied");
    assert_eq!(deliver(&s.pool).await.len(), 1);
}
