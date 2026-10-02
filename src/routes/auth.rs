use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::AppState;
use crate::{
    auth::{AuthUser, hash_password, verify_dummy, verify_password},
    db::PG_UNIQUE_VIOLATION,
    error::AppError,
    validation::normalize_email,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/me", get(me))
}

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    email: String,
    password: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Debug, Serialize, FromRow)]
struct UserResponse {
    id: Uuid,
    email: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct AuthResponse {
    token: String,
    user: UserResponse,
}

async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), AppError> {
    let email = normalize_email(&req.email)?;
    let name = req.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(AppError::BadRequest("名稱長度需為 1–100 字".into()));
    }
    let pw_len = req.password.chars().count();
    if !(8..=128).contains(&pw_len) {
        return Err(AppError::BadRequest("密碼長度需為 8–128 字".into()));
    }

    let password_hash = hash_password(req.password).await?;
    let result = sqlx::query_as::<_, UserResponse>(
        "INSERT INTO users (email, password_hash, name) VALUES ($1, $2, $3)
         RETURNING id, email::text AS email, name",
    )
    .bind(&email)
    .bind(&password_hash)
    .bind(&name)
    .fetch_one(&state.db)
    .await;

    let user = match result {
        Ok(user) => user,
        Err(sqlx::Error::Database(db_err))
            if db_err.code().as_deref() == Some(PG_UNIQUE_VIOLATION) =>
        {
            return Err(AppError::Conflict("此 Email 已註冊".into()));
        }
        Err(err) => return Err(err.into()),
    };

    let token = state.jwt.issue(user.id)?;
    Ok((StatusCode::CREATED, Json(AuthResponse { token, user })))
}

#[derive(Debug, FromRow)]
struct LoginRow {
    id: Uuid,
    email: String,
    name: String,
    password_hash: String,
}

async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let email = req.email.trim();
    let row = sqlx::query_as::<_, LoginRow>(
        "SELECT id, email::text AS email, name, password_hash FROM users WHERE email = $1::citext",
    )
    .bind(email)
    .fetch_optional(&state.db)
    .await?;

    let Some(row) = row else {
        verify_dummy(req.password).await;
        return Err(AppError::Unauthorized);
    };
    if !verify_password(req.password, row.password_hash).await {
        return Err(AppError::Unauthorized);
    }

    let token = state.jwt.issue(row.id)?;
    Ok(Json(AuthResponse {
        token,
        user: UserResponse {
            id: row.id,
            email: row.email,
            name: row.name,
        },
    }))
}

async fn me(State(state): State<AppState>, auth: AuthUser) -> Result<Json<UserResponse>, AppError> {
    sqlx::query_as::<_, UserResponse>(
        "SELECT id, email::text AS email, name FROM users WHERE id = $1",
    )
    .bind(auth.id)
    .fetch_optional(&state.db)
    .await?
    .map(Json)
    .ok_or(AppError::Unauthorized)
}
