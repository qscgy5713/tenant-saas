mod audit_logs;
mod auth;
mod billing;
mod bookings;
mod customers;
mod health;
mod members;
mod public;
mod scheduling;
mod services;
mod tenants;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{ConnectInfo, Request, State},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::PgPool;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::{
    auth::JwtKeys, config::Config, error::AppError, observability, ratelimit::RateLimiter,
};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt: JwtKeys,
    pub public_limiter: Arc<RateLimiter>,
    /// 註冊 / 登入專用,比公開預約端點嚴格得多(防暴力破解與灌帳號)
    pub auth_limiter: Arc<RateLimiter>,
    pub trust_proxy: bool,
    pub public_base_url: String,
    pub cookie: crate::session::CookieConfig,
    pub metrics: Option<PrometheusHandle>,
    pub metrics_token: Option<String>,
    /// 沒設定 Stripe 時為 None:計費端點回 501,前端不顯示升級按鈕
    pub stripe: Option<Arc<crate::billing::Stripe>>,
}

impl AppState {
    pub fn new(db: PgPool, config: &Config) -> Self {
        Self {
            db,
            jwt: JwtKeys::new(config),
            public_limiter: Arc::new(RateLimiter::new(
                config.public_rate_limit_per_min,
                Duration::from_secs(60),
            )),
            auth_limiter: Arc::new(RateLimiter::new(
                config.auth_rate_limit_per_min,
                Duration::from_secs(60),
            )),
            trust_proxy: config.trust_proxy,
            public_base_url: config.public_base_url.clone(),
            cookie: crate::session::CookieConfig::new(config),
            metrics: None,
            metrics_token: config.metrics_token.clone(),
            stripe: config
                .stripe
                .clone()
                .map(|c| Arc::new(crate::billing::Stripe::new(c))),
        }
    }

    pub fn with_metrics(mut self, handle: PrometheusHandle) -> Self {
        self.metrics = Some(handle);
        self
    }
}

/// 限流用的來源識別。
/// 預設用連線的對端位址;只有明確設定信任反向代理時才讀 X-Forwarded-For,
/// 且取最右邊那一筆(最右是由我們自己的代理附加的,左邊的可以被用戶端偽造)。
fn client_key(req: &Request, trust_proxy: bool) -> String {
    if trust_proxy
        && let Some(ip) = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    {
        return ip.to_string();
    }
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_else(|| "unknown".into())
}

async fn rate_limit_public(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    if !state
        .public_limiter
        .check(&client_key(&req, state.trust_proxy))
    {
        return Err(AppError::TooManyRequests);
    }
    Ok(next.run(req).await)
}

async fn rate_limit_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    if !state
        .auth_limiter
        .check(&client_key(&req, state.trust_proxy))
    {
        return Err(AppError::TooManyRequests);
    }
    Ok(next.run(req).await)
}

pub fn router(state: AppState) -> Router {
    let credentials = auth::credential_routes().layer(middleware::from_fn_with_state(
        state.clone(),
        rate_limit_auth,
    ));
    let public = public::routes().layer(middleware::from_fn_with_state(
        state.clone(),
        rate_limit_public,
    ));
    Router::new()
        .route("/health", get(health::health))
        .merge(auth::routes())
        .merge(credentials)
        .merge(tenants::routes())
        .merge(members::routes())
        .merge(services::routes())
        .merge(scheduling::routes())
        .merge(bookings::routes())
        .merge(customers::routes())
        .merge(audit_logs::routes())
        .merge(billing::routes())
        .merge(public)
        .route("/metrics", get(observability::metrics_handler))
        // 越後加的越外層:請求 ID 最外層 → 指標 → 日誌 → 路由
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(observability::make_span)
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(middleware::from_fn(observability::track_metrics))
        .layer(middleware::from_fn(observability::request_id))
        .with_state(state)
}
