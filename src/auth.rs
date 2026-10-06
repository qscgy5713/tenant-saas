use std::sync::LazyLock;

use anyhow::{Context, anyhow};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use axum::{extract::FromRequestParts, http::request::Parts};
use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{config::Config, error::AppError, routes::AppState};

#[derive(Clone)]
pub struct JwtKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
    ttl_secs: u64,
}

impl JwtKeys {
    pub fn new(config: &Config) -> Self {
        let secret = config.jwt_secret.as_bytes();
        Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
            ttl_secs: config.jwt_ttl_secs,
        }
    }

    pub fn issue(&self, user_id: Uuid) -> Result<String, AppError> {
        self.issue_at(user_id, jsonwebtoken::get_current_timestamp())
    }

    /// 指定簽發時間,方便測試過期情境。
    pub fn issue_at(&self, user_id: Uuid, now: u64) -> Result<String, AppError> {
        let claims = Claims {
            sub: user_id,
            iat: now,
            exp: now + self.ttl_secs,
        };
        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .context("簽發 JWT 失敗")
            .map_err(AppError::from)
    }

    fn verify(&self, token: &str) -> Result<Claims, AppError> {
        let validation = Validation::new(Algorithm::HS256);
        decode::<Claims>(token, &self.decoding, &validation)
            .map(|data| data.claims)
            .map_err(|_| AppError::Unauthorized)
    }
}

/// (上次改密碼, 上次「登出所有裝置」)
type Cutoffs = (Option<DateTime<Utc>>, Option<DateTime<Utc>>);

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: Uuid,
    iat: u64,
    exp: u64,
}

/// 通過 JWT 驗證的使用者。租戶不放在 token 內,之後每個請求另外驗證成員資格。
#[derive(Debug, Clone, Copy)]
pub struct AuthUser {
    pub id: Uuid,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Bearer(測試、腳本)優先;瀏覽器走 cookie。cookie 是瀏覽器自動帶上的,
        // 所以用 cookie 認證的寫入請求要多檢查 Origin(CSRF),Bearer 不用
        let bearer = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        let token = match bearer {
            Some(h) => h.strip_prefix("Bearer ").ok_or(AppError::Unauthorized)?,
            None => {
                let cookie = state
                    .cookie
                    .read(&parts.headers)
                    .ok_or(AppError::Unauthorized)?;
                if !state.cookie.origin_ok(&parts.method, &parts.headers) {
                    tracing::warn!(method = %parts.method, "拒絕 Origin 不在白名單內的 cookie 請求");
                    return Err(AppError::Forbidden);
                }
                cookie
            }
        };
        let claims = state.jwt.verify(token)?;

        // JWT 本身無法撤銷。每次多查一次主鍵:使用者還在嗎?簽發時間有沒有早於上次改密碼?
        // 重設密碼後,舊的(可能被盜的)登入因此立刻失效;被刪除的使用者的 token 也不再有效。
        // 精度:簽發時間只到秒,改密碼後同一秒內簽發的舊 token 不會被擋(約 1 秒的窗口)。
        // 「登出所有裝置」(sessions_revoked_at)同理,但**同一秒內簽發的也拒絕**:撤銷要寧可多擋,
        // 不能留下「撤銷的那一秒內簽發的 token 仍有效」的縫(代價:登出後 1 秒內立刻重新登入會被擋,重試即可)。
        // 改密碼則維持「早於才拒絕」:重設密碼後馬上用新密碼登入是正常流程,不能擋。
        let cutoffs: Option<Cutoffs> = sqlx::query_as(
            "SELECT password_changed_at, sessions_revoked_at FROM users WHERE id = $1",
        )
        .bind(claims.sub)
        .fetch_optional(&state.db)
        .await?;
        let Some((changed, revoked)) = cutoffs else {
            return Err(AppError::Unauthorized);
        };
        let iat = claims.iat as i64;
        if changed.is_some_and(|at| iat < at.timestamp())
            || revoked.is_some_and(|at| iat <= at.timestamp())
        {
            return Err(AppError::Unauthorized);
        }
        Ok(AuthUser { id: claims.sub })
    }
}

pub async fn hash_password(password: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(|e| anyhow!("密碼雜湊失敗: {e}"))
    })
    .await
    .map_err(|e| AppError::Internal(e.into()))?
    .map_err(AppError::Internal)
}

pub async fn verify_password(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash)
            .map(|parsed| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok()
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

/// 帳號不存在時也對這個假雜湊做一次驗證,讓回應時間與「密碼錯誤」接近,避免洩漏哪些 Email 已註冊。
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
    Argon2::default()
        .hash_password(b"dummy-password-for-timing")
        .expect("產生假雜湊失敗")
        .to_string()
});

pub async fn verify_dummy(password: String) {
    verify_password(password, DUMMY_HASH.clone()).await;
}
