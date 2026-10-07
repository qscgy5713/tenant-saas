use std::sync::{Arc, Mutex, OnceLock};

use crate::common::{app, call, create_service, create_tenant, signup, test_config};
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use serde_json::json;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tenant_saas::{
    config::Config,
    mail::{Mailer, MemoryMailer},
    routes::{self, AppState},
    worker::tick,
};
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;

/// 整個測試程式共用一個 recorder(全域,只能安裝一次)
fn recorder() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| {
            PrometheusBuilder::new()
                .install_recorder()
                .expect("安裝 recorder")
        })
        .clone()
}

const METRICS_TOKEN: &str = "metrics-token-0123456789";

fn app_with_metrics(pool: PgPool, token: Option<&str>) -> Router {
    let config = Config {
        metrics_token: token.map(str::to_owned),
        ..test_config(3600)
    };
    routes::router(AppState::new(pool, &config).with_metrics(recorder()))
}

async fn raw(app: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn get(uri: &str) -> axum::http::request::Builder {
    Request::builder().method(Method::GET).uri(uri)
}

// ---------- 請求 ID ----------

#[sqlx::test]
async fn request_ids_are_generated_echoed_and_sanitized(pool: PgPool) {
    let app = app(pool);

    let (_, headers, _) = raw(&app, get("/health").body(Body::empty()).unwrap()).await;
    let generated = headers["x-request-id"].to_str().unwrap();
    assert!(
        uuid::Uuid::parse_str(generated).is_ok(),
        "沒帶時自動產生 UUID: {generated}"
    );

    // 合法的沿用(讓上游代理 / 前端可以串起整條請求)
    let (_, headers, _) = raw(
        &app,
        get("/health")
            .header("x-request-id", "trace-abc_123")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(headers["x-request-id"], "trace-abc_123");

    // 不合法的(含空白、引號、過長)一律換掉,不能進日誌
    for bad in ["has space", "quo\"te", &"a".repeat(65)] {
        let (_, headers, _) = raw(
            &app,
            get("/health")
                .header("x-request-id", bad)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        let id = headers["x-request-id"].to_str().unwrap();
        assert_ne!(id, bad);
        assert!(uuid::Uuid::parse_str(id).is_ok());
    }

    // 錯誤回應也帶請求 ID(排查問題時要用)
    let (status, headers, _) = raw(&app, get("/auth/me").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(headers.contains_key("x-request-id"));
}

// ---------- /metrics ----------

#[sqlx::test]
async fn metrics_endpoint_is_off_by_default_and_protected_when_on(pool: PgPool) {
    // 沒設定 token:端點不存在
    let off = app_with_metrics(pool.clone(), None);
    assert_eq!(
        raw(&off, get("/metrics").body(Body::empty()).unwrap())
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        raw(
            &off,
            get("/metrics")
                .header(header::AUTHORIZATION, format!("Bearer {METRICS_TOKEN}"))
                .body(Body::empty())
                .unwrap()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    let on = app_with_metrics(pool, Some(METRICS_TOKEN));
    assert_eq!(
        raw(&on, get("/metrics").body(Body::empty()).unwrap())
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    for bad in ["Bearer wrong", "Bearer ", "Basic abc", METRICS_TOKEN] {
        let req = get("/metrics")
            .header(header::AUTHORIZATION, bad)
            .body(Body::empty())
            .unwrap();
        assert_eq!(raw(&on, req).await.0, StatusCode::UNAUTHORIZED, "{bad}");
    }
    let req = get("/metrics")
        .header(header::AUTHORIZATION, format!("Bearer {METRICS_TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let (status, headers, _) = raw(&on, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
}

#[sqlx::test]
async fn metrics_use_route_templates_and_never_contain_tokens(pool: PgPool) {
    let app = app_with_metrics(pool.clone(), Some(METRICS_TOKEN));
    let secret = "SECRETMETRICTOKEN0123456789abcdefghijklmnopqrstuvwxyz0123456789ABCD";

    // 帶著 token 的路徑、不存在的路徑、一般路徑
    for uri in [
        format!("/public/bookings/{secret}"),
        "/no/such/route/xyz".to_string(),
        "/health".to_string(),
    ] {
        raw(&app, get(&uri).body(Body::empty()).unwrap()).await;
    }
    let req = get("/metrics")
        .header(header::AUTHORIZATION, format!("Bearer {METRICS_TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let (_, _, body) = raw(&app, req).await;

    assert!(!body.contains(secret), "指標不可含實際網址中的 token");
    assert!(
        !body.contains("no/such/route"),
        "不存在的路徑不可變成 label(否則攻擊者能無限製造 label)"
    );
    assert!(
        body.contains(r#"path="/public/bookings/{token}""#),
        "應使用路由樣板"
    );
    assert!(body.contains(r#"path="unmatched""#));
    assert!(body.contains("http_requests_total{") && body.contains(r#"status="404""#));
    assert!(body.contains("http_request_duration_seconds"));
    assert!(body.contains("app_db_up 1"));
}

#[sqlx::test]
async fn metrics_report_the_mail_queue(pool: PgPool) {
    let app = app_with_metrics(pool.clone(), Some(METRICS_TOKEN));
    let owner = signup(&app, "a@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    let scrape = || {
        let app = app.clone();
        async move {
            let req = get("/metrics")
                .header(header::AUTHORIZATION, format!("Bearer {METRICS_TOKEN}"))
                .body(Body::empty())
                .unwrap();
            raw(&app, req).await.2
        }
    };
    let value = |body: &str, name: &str| -> f64 {
        body.lines()
            .find(|l| l.starts_with(&format!("{name} ")))
            .unwrap_or_else(|| panic!("找不到 {name}\n{body}"))
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap()
    };

    let before = scrape().await;
    assert_eq!(value(&before, "email_outbox_pending"), 0.0);

    // 排兩封信:一封到期很久、一封還沒到期
    call(
        &app,
        Method::POST,
        "/t/shop-a/invitations",
        Some(json!({"email": "x@example.com", "role": "staff"})),
        Some(&owner),
    )
    .await;
    sqlx::query("UPDATE email_outbox SET run_at = now() - interval '90 seconds'")
        .execute(&pool)
        .await
        .unwrap();
    let tenant: uuid::Uuid = sqlx::query_scalar("SELECT id FROM tenants WHERE slug = 'shop-a'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO email_outbox (tenant_id, to_email, subject, body, run_at) VALUES ($1, 'y@example.com', 's', 'b', now() + interval '1 hour')")
        .bind(tenant).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO email_outbox (tenant_id, to_email, subject, body, status) VALUES ($1, 'z@example.com', 's', '', 'failed')")
        .bind(tenant).execute(&pool).await.unwrap();

    let body = scrape().await;
    assert_eq!(value(&body, "email_outbox_pending"), 2.0);
    assert_eq!(value(&body, "email_outbox_failed"), 1.0);
    let lag = value(&body, "email_outbox_oldest_due_age_seconds");
    assert!(
        (89.0..=120.0).contains(&lag),
        "只算已到期的最舊那封,未到期的不算積壓: {lag}"
    );

    // 寄完之後歸零
    sqlx::query("UPDATE email_outbox SET run_at = now() WHERE status = 'pending'")
        .execute(&pool)
        .await
        .unwrap();
    tick(
        &pool,
        &Mailer::Memory(MemoryMailer::new()),
        "http://app.test",
    )
    .await
    .unwrap();
    let body = scrape().await;
    assert_eq!(value(&body, "email_outbox_pending"), 0.0);
    assert_eq!(value(&body, "email_outbox_oldest_due_age_seconds"), 0.0);
}

// ---------- 日誌不洩漏 token ----------

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captured {
    type Writer = Captured;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// 單執行緒 runtime + 區域訂閱者:請求的日誌都在這個執行緒上產生,可以完整攔截
#[tokio::test]
async fn request_logs_use_route_templates_and_include_the_request_id() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    // 不連資料庫:這個 token 太短,在查資料庫之前就回 404
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://nobody:nobody@127.0.0.1:1/none")
        .unwrap();
    let app = routes::router(AppState::new(pool, &test_config(3600)));

    let secret = "SECRETLOG12";
    let (status, headers, _) = raw(
        &app,
        get(&format!("/public/bookings/{secret}"))
            .header("x-request-id", "req-42")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(headers["x-request-id"], "req-42");

    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(!output.is_empty(), "應該有攔截到日誌");
    assert!(
        !output.contains(secret),
        "日誌不可含網址中的 token:\n{output}"
    );
    assert!(
        output.contains("path=/public/bookings/{token}"),
        "應記錄路由樣板:\n{output}"
    );
    assert!(
        output.contains("request_id=req-42"),
        "日誌要帶請求 ID:\n{output}"
    );
    assert!(
        output.contains("status=404") || output.contains("status: 404") || output.contains("404"),
        "{output}"
    );
}

#[sqlx::test]
async fn services_still_work_end_to_end_with_the_new_layers(pool: PgPool) {
    // 加了三層 middleware 之後,一般流程不受影響,而且每個回應都帶請求 ID
    let app = app_with_metrics(pool, Some(METRICS_TOKEN));
    let owner = signup(&app, "a@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    let svc = create_service(&app, &owner, "shop-a", "剪髮").await;
    assert!(svc["id"].is_string());
    let req = get("/t/shop-a/services")
        .header(header::AUTHORIZATION, format!("Bearer {owner}"))
        .body(Body::empty())
        .unwrap();
    let (status, headers, _) = raw(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.contains_key("x-request-id"));
}
