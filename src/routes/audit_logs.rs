//! 稽核日誌查詢(owner / manager)。唯讀:沒有任何修改或刪除的 API,資料庫也沒給這些權限。

use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::AppState;
use crate::{error::AppError, tenancy::TenantCtx};

pub fn routes() -> Router<AppState> {
    Router::new().route("/t/{slug}/audit-logs", get(list))
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    /// 動作前綴,例如 `booking.` 或完整的 `booking.cancelled`
    action: Option<String>,
    entity_type: Option<String>,
    entity_id: Option<Uuid>,
    actor_user_id: Option<Uuid>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    /// 游標:只取 id 小於此值的紀錄(由上一頁回傳的 `next_before` 帶入)
    before: Option<i64>,
    limit: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
struct Entry {
    id: i64,
    created_at: DateTime<Utc>,
    actor_type: String,
    actor_user_id: Option<Uuid>,
    /// 目前的姓名(使用者離開店家後仍保有 id,但姓名可能查不到)
    actor_name: Option<String>,
    action: String,
    entity_type: String,
    entity_id: Option<Uuid>,
    detail: Value,
}

#[derive(Debug, Serialize)]
struct Page {
    items: Vec<Entry>,
    next_before: Option<i64>,
}

async fn list(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Query(q): Query<ListQuery>,
) -> Result<Json<Page>, AppError> {
    ctx.require_manager()?;
    let limit = q.limit.unwrap_or(50).clamp(1, 100);
    if q.action.as_ref().is_some_and(|a| a.len() > 64) {
        return Err(AppError::BadRequest("action 過長".into()));
    }

    let mut tx = ctx.begin(&state).await?;
    // 多抓一筆判斷有沒有下一頁。前綴比對用 left(),避免使用者輸入的 % _ 被當成 LIKE 萬用字元
    let mut items = sqlx::query_as::<_, Entry>(
        "SELECT a.id, a.created_at, a.actor_type, a.actor_user_id, u.name AS actor_name,
                a.action, a.entity_type, a.entity_id, a.detail
         FROM audit_logs a LEFT JOIN users u ON u.id = a.actor_user_id
         WHERE ($1::text IS NULL OR left(a.action, length($1)) = $1)
           AND ($2::text IS NULL OR a.entity_type = $2)
           AND ($3::uuid IS NULL OR a.entity_id = $3)
           AND ($4::uuid IS NULL OR a.actor_user_id = $4)
           AND ($5::timestamptz IS NULL OR a.created_at >= $5)
           AND ($6::timestamptz IS NULL OR a.created_at < $6)
           AND ($7::bigint IS NULL OR a.id < $7)
         ORDER BY a.id DESC LIMIT $8",
    )
    .bind(&q.action)
    .bind(&q.entity_type)
    .bind(q.entity_id)
    .bind(q.actor_user_id)
    .bind(q.from)
    .bind(q.to)
    .bind(q.before)
    .bind(limit + 1)
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;

    let next_before = if items.len() as i64 > limit {
        items.truncate(limit as usize);
        items.last().map(|e| e.id)
    } else {
        None
    };
    Ok(Json(Page { items, next_before }))
}
