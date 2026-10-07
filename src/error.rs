use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("未登入或憑證無效")]
    Unauthorized,
    #[error("沒有權限執行此操作")]
    Forbidden,
    #[error("請先驗證 Email 才能建立店家(請查看信箱;沒收到可以在店家列表重新寄送)")]
    EmailNotVerified,
    #[error("找不到資源")]
    NotFound,
    #[error("請求過於頻繁,請稍後再試")]
    TooManyRequests,
    /// 超過訂閱方案的限額(402 Payment Required)
    #[error("{0}")]
    LimitReached(String),
    #[error("{0}")]
    Conflict(String),
    /// 這個功能沒有啟用(例如沒設定 Stripe)
    #[error("{0}")]
    NotEnabled(String),
    /// 上游服務(Stripe)失敗
    #[error("{0}")]
    Upstream(String),
    #[error("內部錯誤")]
    Internal(#[source] anyhow::Error),
}

/// PostgreSQL `character_not_in_repertoire`:文字裡有資料庫存不了的字元(實際上就是 NUL,`\u0000`)
const PG_CHARACTER_NOT_IN_REPERTOIRE: &str = "22021";

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        // 使用者輸入的任何文字欄位(包含不需登入的註冊、公開預約)都可能夾帶 NUL。
        // 那是輸入錯誤,不是伺服器故障:回 400,不要變成 500 —— 否則任何人都能觸發 5xx 告警。
        // 在這裡統一處理,不必在每個欄位各自檢查,之後新增的欄位也自動涵蓋
        if crate::db::pg_code(&err).as_deref() == Some(PG_CHARACTER_NOT_IN_REPERTOIRE) {
            return AppError::BadRequest("內容含有不允許的字元".into());
        }
        AppError::Internal(err.into())
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::Forbidden => StatusCode::FORBIDDEN,
            AppError::EmailNotVerified => StatusCode::FORBIDDEN,
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            AppError::LimitReached(_) => StatusCode::PAYMENT_REQUIRED,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::NotEnabled(_) => StatusCode::NOT_IMPLEMENTED,
            AppError::Upstream(_) => StatusCode::BAD_GATEWAY,
            AppError::Internal(err) => {
                tracing::error!(error = ?err, "內部錯誤");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}
