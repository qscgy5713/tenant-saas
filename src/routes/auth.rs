use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    response::{AppendHeaders, IntoResponse},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::AppState;
use crate::{
    auth::{AuthUser, hash_password, verify_dummy, verify_password},
    db::{PG_UNIQUE_VIOLATION, pg_code},
    error::AppError,
    mail, token,
    validation::normalize_email,
};

/// 註冊與登入:可被暴力嘗試 / 灌帳號,所以另外掛限流(見 routes/mod.rs)
pub fn credential_routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/forgot-password", post(forgot_password))
        .route("/auth/reset-password", post(reset_password))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/me", get(me))
        .route("/auth/logout", post(logout))
}

/// 登入 / 註冊成功的回應:body 給非瀏覽器客戶端(Bearer),瀏覽器用 `Set-Cookie`。
/// 帶憑證的回應不可被快取。
fn with_session(
    state: &AppState,
    status: StatusCode,
    auth: AuthResponse,
) -> impl IntoResponse + use<> {
    let cookie = state.cookie.set(&auth.token);
    (
        status,
        AppendHeaders([
            (header::SET_COOKIE, cookie),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ]),
        Json(auth),
    )
}

/// 登出:HttpOnly 的 cookie 前端的 JS 刪不掉,只能由伺服器清除。
/// 不需要登入(cookie 過期了也要能登出),也不檢查 Origin:最糟只是被強制登出。
/// JWT 本身無法撤銷,偷到 token 的人在到期前仍可用 Bearer;改密碼會讓所有舊 token 失效。
async fn logout(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::NO_CONTENT,
        AppendHeaders([
            (header::SET_COOKIE, state.cookie.clear()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ]),
    )
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
) -> Result<impl IntoResponse, AppError> {
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
    Ok(with_session(
        &state,
        StatusCode::CREATED,
        AuthResponse { token, user },
    ))
}

#[derive(Debug, FromRow)]
struct LoginRow {
    id: Uuid,
    email: String,
    name: String,
    password_hash: String,
    locked: bool,
}

/// 登入。帳號不存在、密碼錯誤、帳號鎖定中,對外一律是同一個 401(不洩漏哪些 Email 已註冊 / 是否被鎖);
/// 三種情況都會做一次雜湊運算,耗時也相近。
/// 連續失敗 5 次鎖 15 分鐘(見 migration 0017);鎖定期間連正確密碼都不收。
async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    let email = req.email.trim();
    let row = sqlx::query_as::<_, LoginRow>(
        "SELECT id, email::text AS email, name, password_hash,
                COALESCE(locked_until > now(), false) AS locked
         FROM users WHERE email = $1::citext",
    )
    .bind(email)
    .fetch_optional(&state.db)
    .await?;

    let Some(row) = row else {
        verify_dummy(req.password).await;
        return Err(AppError::Unauthorized);
    };
    let password_ok = verify_password(req.password, row.password_hash).await;
    if row.locked {
        metrics::counter!("login_rejected_locked_total").increment(1);
        return Err(AppError::Unauthorized);
    }
    if !password_ok {
        let mail = mail::account_locked(&row.email);
        let newly_locked = sqlx::query_scalar::<_, bool>("SELECT login_failed($1, $2, $3)")
            .bind(row.id)
            .bind(&mail.subject)
            .bind(&mail.body)
            .fetch_one(&state.db)
            .await?;
        if newly_locked {
            // 日誌只記使用者 ID,不記 Email
            tracing::warn!(user_id = %row.id, "帳號因連續登入失敗被鎖定");
            metrics::counter!("account_locked_total").increment(1);
        }
        return Err(AppError::Unauthorized);
    }
    sqlx::query("SELECT login_succeeded($1)")
        .bind(row.id)
        .execute(&state.db)
        .await?;

    let token = state.jwt.issue(row.id)?;
    Ok(with_session(
        &state,
        StatusCode::OK,
        AuthResponse {
            token,
            user: UserResponse {
                id: row.id,
                email: row.email,
                name: row.name,
            },
        },
    ))
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

#[derive(Debug, Deserialize)]
struct ForgotRequest {
    email: String,
}

#[derive(Debug, Serialize)]
struct Accepted {
    message: &'static str,
}

/// 申請重設密碼。**不論 Email 是否註冊都回一模一樣的 202**,不洩漏哪些 Email 已註冊;
/// 同一個帳號每小時最多 3 封(資料庫函式內控制),避免被拿來灌爆別人的信箱。
async fn forgot_password(
    State(state): State<AppState>,
    Json(req): Json<ForgotRequest>,
) -> Result<(StatusCode, Json<Accepted>), AppError> {
    let email = normalize_email(&req.email)?;

    // 信件內容永遠先組好(含原始 token),由函式決定要不要寫入 outbox。
    // 這樣「Email 不存在」與「存在」在應用程式端做的事相同。
    let raw = token::generate();
    let link = mail::password_reset_link(&state.public_base_url, &raw);
    let body = mail::password_reset(&email, &link);

    sqlx::query_scalar::<_, bool>("SELECT request_password_reset($1::citext, $2, $3, $4)")
        .bind(&email)
        .bind(token::hash(&raw))
        .bind(&body.subject)
        .bind(&body.body)
        .fetch_one(&state.db)
        .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(Accepted {
            message: "如果這個 Email 已註冊,重設密碼的信已經寄出。請查看信箱(包含垃圾郵件匣)。",
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct ResetRequest {
    token: String,
    password: String,
}

const PG_NO_DATA_FOUND: &str = "P0002";

/// 用信中的 token 設定新密碼。token 無效、過期、已使用都是同一個錯誤。
/// 成功後這個帳號所有舊的登入立刻失效(見 `AuthUser`)。
async fn reset_password(
    State(state): State<AppState>,
    Json(req): Json<ResetRequest>,
) -> Result<Json<Accepted>, AppError> {
    let token = req.token.trim();
    let invalid = || AppError::BadRequest("重設連結無效或已過期,請重新申請".into());
    if !(16..=128).contains(&token.len()) {
        return Err(invalid());
    }
    let pw_len = req.password.chars().count();
    if !(8..=128).contains(&pw_len) {
        return Err(AppError::BadRequest("密碼長度需為 8–128 字".into()));
    }

    let password_hash = hash_password(req.password).await?;
    let result = sqlx::query_scalar::<_, Uuid>("SELECT reset_password($1, $2)")
        .bind(token::hash(token))
        .bind(&password_hash)
        .fetch_one(&state.db)
        .await;
    match result {
        Ok(_) => Ok(Json(Accepted {
            message: "密碼已更新,請用新密碼登入。",
        })),
        Err(e) if pg_code(&e).as_deref() == Some(PG_NO_DATA_FOUND) => Err(invalid()),
        Err(e) => Err(e.into()),
    }
}
