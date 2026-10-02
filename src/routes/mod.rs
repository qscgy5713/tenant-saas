mod auth;
mod bookings;
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
use sqlx::PgPool;
use tower_http::trace::TraceLayer;

use crate::{auth::JwtKeys, config::Config, error::AppError, ratelimit::RateLimiter};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt: JwtKeys,
    pub public_limiter: Arc<RateLimiter>,
    pub trust_proxy: bool,
    pub public_base_url: String,
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
            trust_proxy: config.trust_proxy,
            public_base_url: config.public_base_url.clone(),
        }
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

pub fn router(state: AppState) -> Router {
    let public = public::routes().layer(middleware::from_fn_with_state(
        state.clone(),
        rate_limit_public,
    ));
    Router::new()
        .route("/health", get(health::health))
        .merge(auth::routes())
        .merge(tenants::routes())
        .merge(members::routes())
        .merge(services::routes())
        .merge(scheduling::routes())
        .merge(bookings::routes())
        .merge(public)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
