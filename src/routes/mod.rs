mod auth;
mod health;
mod members;
mod scheduling;
mod services;
mod tenants;

use axum::{Router, routing::get};
use sqlx::PgPool;
use tower_http::trace::TraceLayer;

use crate::{auth::JwtKeys, config::Config};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt: JwtKeys,
}

impl AppState {
    pub fn new(db: PgPool, config: &Config) -> Self {
        Self {
            db,
            jwt: JwtKeys::new(config),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .merge(auth::routes())
        .merge(tenants::routes())
        .merge(members::routes())
        .merge(services::routes())
        .merge(scheduling::routes())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
