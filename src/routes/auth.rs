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
        .route("/auth/verify-email", post(verify_email))
        // 要登入 + 重新輸入密碼:放在這裡是為了套用每 IP 的限流(防止拿著被盜的登入狀態試密碼)
        .route("/auth/delete-account", post(delete_account))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/me", get(me))
        .route("/auth/logout", post(logout))
        .route("/auth/logout-all", post(logout_all))
        .route("/auth/security-events", get(security_events))
        .route("/auth/resend-verification", post(resend_verification))
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
    /// 沒驗證 Email 就不能建立店家(其他功能不受影響)
    email_verified: bool,
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
        // 關掉驗證要求時(只有開發 / 測試能關),新使用者直接視為已驗證、也不寄驗證信
        "INSERT INTO users (email, password_hash, name, email_verified_at)
         VALUES ($1, $2, $3, CASE WHEN $4 THEN NULL ELSE now() END)
         RETURNING id, email::text AS email, name, email_verified_at IS NOT NULL AS email_verified",
    )
    .bind(&email)
    .bind(&password_hash)
    .bind(&name)
    .bind(state.require_verified_email)
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

    metrics::counter!("registrations_total").increment(1);
    record_event(&state, user.id, "registered").await;
    // 驗證信寄不出去不該讓註冊失敗(使用者之後可以重寄),但要留下紀錄
    if state.require_verified_email
        && let Err(e) = send_verification(&state, user.id).await
    {
        tracing::error!(error = ?e, user_id = %user.id, "註冊後寫入驗證信失敗");
    }

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
    email_verified: bool,
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
                COALESCE(locked_until > now(), false) AS locked,
                email_verified_at IS NOT NULL AS email_verified
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
        record_event(&state, row.id, "login_failed").await;
        if newly_locked {
            record_event(&state, row.id, "locked").await;
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
    record_event(&state, row.id, "login").await;

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
                email_verified: row.email_verified,
            },
        },
    ))
}

async fn me(State(state): State<AppState>, auth: AuthUser) -> Result<Json<UserResponse>, AppError> {
    sqlx::query_as::<_, UserResponse>(
        "SELECT id, email::text AS email, name, email_verified_at IS NOT NULL AS email_verified
         FROM users WHERE id = $1",
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
        Ok(user_id) => {
            record_event(&state, user_id, "password_reset").await;
            Ok(Json(Accepted {
                message: "密碼已更新,請用新密碼登入。",
            }))
        }
        Err(e) if pg_code(&e).as_deref() == Some(PG_NO_DATA_FOUND) => Err(invalid()),
        Err(e) => Err(e.into()),
    }
}

/// 產生驗證 token、組信,交給資料庫函式決定要不要寄(已驗證 / 這小時寄太多次就不寄)
async fn send_verification(state: &AppState, user_id: Uuid) -> Result<String, AppError> {
    let email: String = sqlx::query_scalar("SELECT email::text FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
    let raw = token::generate();
    let link = mail::email_verification_link(&state.public_base_url, &raw);
    let msg = mail::email_verification(&email, &link);
    let result = sqlx::query_scalar("SELECT request_email_verification($1, $2, $3, $4)")
        .bind(user_id)
        .bind(token::hash(&raw))
        .bind(&msg.subject)
        .bind(&msg.body)
        .fetch_one(&state.db)
        .await?;
    Ok(result)
}

/// 重寄驗證信。已驗證回 409;這小時已寄 3 封回 429。
async fn resend_verification(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<(StatusCode, Json<Accepted>), AppError> {
    match send_verification(&state, auth.id).await?.as_str() {
        "sent" => Ok((
            StatusCode::ACCEPTED,
            Json(Accepted {
                message: "驗證信已寄出,請查看信箱(包含垃圾郵件匣)。",
            }),
        )),
        "already_verified" => Err(AppError::Conflict("這個 Email 已經驗證過了".into())),
        "throttled" => Err(AppError::TooManyRequests),
        _ => Err(AppError::Unauthorized),
    }
}

#[derive(Debug, Deserialize)]
struct VerifyRequest {
    token: String,
}

/// 用信中的 token 驗證 Email。**不需要登入**:使用者常在另一台裝置 / 另一個瀏覽器開信。
/// token 無效、過期、已使用都是同一個錯誤。
async fn verify_email(
    State(state): State<AppState>,
    Json(req): Json<VerifyRequest>,
) -> Result<Json<Accepted>, AppError> {
    let token = req.token.trim();
    let invalid = || AppError::BadRequest("驗證連結無效或已過期,請登入後重新寄送".into());
    if !(16..=128).contains(&token.len()) {
        return Err(invalid());
    }
    let result = sqlx::query_scalar::<_, Uuid>("SELECT verify_email($1)")
        .bind(token::hash(token))
        .fetch_one(&state.db)
        .await;
    match result {
        Ok(user_id) => {
            record_event(&state, user_id, "email_verified").await;
            Ok(Json(Accepted {
                message: "Email 已驗證,現在可以建立店家了。",
            }))
        }
        Err(e) if pg_code(&e).as_deref() == Some(PG_NO_DATA_FOUND) => Err(invalid()),
        Err(e) => Err(e.into()),
    }
}

/// 登出所有裝置:這個時間點之前簽發的 token 全部失效(包含目前這個),並清掉這個瀏覽器的 cookie。
async fn logout_all(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    sqlx::query("SELECT revoke_sessions($1)")
        .bind(auth.id)
        .execute(&state.db)
        .await?;
    record_event(&state, auth.id, "logout_all").await;
    Ok((
        StatusCode::NO_CONTENT,
        AppendHeaders([
            (header::SET_COOKIE, state.cookie.clear()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ]),
    ))
}

#[derive(Debug, Deserialize)]
struct DeleteAccountRequest {
    password: String,
}

/// 刪除自己的帳號(不可還原)。要重新輸入密碼,防止被盜用的登入狀態直接把帳號刪掉。
/// 不是刪除資料列而是匿名化(見 migration 0030):Email 釋出、名稱與密碼抹除、所有登入失效、成員資格停用。
/// 還是某家店的擁有者、或還有負責的未來預約時不能刪。
/// 密碼錯誤回 400 而不是 401:前端對 401 會直接當成登入過期而登出。
async fn delete_account(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<DeleteAccountRequest>,
) -> Result<impl IntoResponse, AppError> {
    confirm_password(&state, auth.id, req.password).await?;
    let result = sqlx::query("SELECT delete_account($1)")
        .bind(auth.id)
        .execute(&state.db)
        .await;
    if let Err(e) = result {
        return Err(match pg_code(&e).as_deref() {
            Some("P0002") => AppError::Conflict(
                "你還是某家店的擁有者,請先刪除那家店(設定頁 → 刪除店家)再刪除帳號".into(),
            ),
            Some("P0001") => AppError::Conflict(
                "你還有負責的未來預約,請先請店家改派或取消這些預約再刪除帳號".into(),
            ),
            Some("P0003") => AppError::Conflict("這個帳號已經刪除過了".into()),
            _ => e.into(),
        });
    }
    tracing::info!(user_id = %auth.id, "使用者帳號已刪除(匿名化)");
    Ok((
        StatusCode::NO_CONTENT,
        AppendHeaders([
            (header::SET_COOKIE, state.cookie.clear()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ]),
    ))
}

/// 記一筆帳號層級的安全事件。**記錄失敗不能讓登入 / 註冊等本來的動作失敗**,只留日誌
async fn record_event(state: &AppState, user_id: Uuid, kind: &str) {
    if let Err(e) = sqlx::query("SELECT record_account_event($1, $2)")
        .bind(user_id)
        .bind(kind)
        .execute(&state.db)
        .await
    {
        tracing::error!(error = %e, %user_id, kind, "記錄帳號事件失敗");
    }
}

#[derive(Debug, Serialize, FromRow)]
struct SecurityEvent {
    kind: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// 我的最近帳號活動(只看得到自己的,最新的在前,最多 50 筆)。
/// 用途:發現不是自己的登入或一直失敗的嘗試,就知道該改密碼 / 登出所有裝置。
async fn security_events(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let events: Vec<SecurityEvent> =
        sqlx::query_as("SELECT kind, created_at FROM list_account_events($1, 50)")
            .bind(auth.id)
            .fetch_all(&state.db)
            .await?;
    Ok((
        AppendHeaders([(header::CACHE_CONTROL, "no-store".to_string())]),
        Json(events),
    ))
}

/// 「重新輸入密碼」的確認(刪除帳號、移交店主…):**失敗與登入共用同一套計數與鎖定**。
///
/// 這些端點本來就是為了防「被盜用的登入狀態」做危險操作,如果不計入鎖定,持有登入狀態的人就能無限次猜密碼
/// (猜中了就能刪帳號 / 把店送走)。連續失敗 5 次會鎖定帳號 15 分鐘(登入也一併被擋),並寄通知信。
/// 錯誤回 400 而不是 401:前端對 401 一律當成登入過期而登出。
pub(crate) async fn confirm_password(
    state: &AppState,
    user_id: Uuid,
    password: String,
) -> Result<(), AppError> {
    let (hash, email, locked): (String, String, bool) = sqlx::query_as(
        "SELECT password_hash, email::text, COALESCE(locked_until > now(), false)
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;
    if locked {
        // 鎖定中連正確的密碼都不收(和登入一致),也不繼續計數
        return Err(AppError::BadRequest(
            "密碼嘗試次數過多,帳號暫時鎖定,請稍後再試".into(),
        ));
    }
    if !verify_password(password, hash).await {
        let mail = mail::account_locked(&email);
        let newly_locked = sqlx::query_scalar::<_, bool>("SELECT login_failed($1, $2, $3)")
            .bind(user_id)
            .bind(&mail.subject)
            .bind(&mail.body)
            .fetch_one(&state.db)
            .await?;
        if newly_locked {
            tracing::warn!(%user_id, "帳號因重新確認密碼連續失敗被鎖定");
            metrics::counter!("account_locked_total").increment(1);
            record_event(state, user_id, "locked").await;
        }
        return Err(AppError::BadRequest("密碼不正確".into()));
    }
    // 成功就清掉先前的失敗次數(和登入一致)
    sqlx::query("SELECT login_succeeded($1)")
        .bind(user_id)
        .execute(&state.db)
        .await?;
    Ok(())
}
