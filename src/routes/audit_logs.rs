//! 稽核日誌查詢(owner / manager)。唯讀:沒有任何修改或刪除的 API,資料庫也沒給這些權限。

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderName, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::AppState;
use crate::{
    audit::{self, Actor},
    csv::{self, EXPORT_MAX_ROWS},
    error::AppError,
    tenancy::TenantCtx,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/t/{slug}/audit-logs", get(list))
        .route("/t/{slug}/audit-logs/export.csv", get(export))
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

#[derive(Debug, Deserialize)]
struct ExportQuery {
    action: Option<String>,
    entity_type: Option<String>,
    entity_id: Option<Uuid>,
    actor_user_id: Option<Uuid>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct ExportRow {
    id: i64,
    created_at: DateTime<Utc>,
    /// 店家時區的當地時間(給看報表的人;UTC 欄位另外保留給程式處理)
    created_local: String,
    actor_type: String,
    actor_user_id: Option<Uuid>,
    actor_name: Option<String>,
    action: String,
    entity_type: String,
    entity_id: Option<Uuid>,
    detail: Value,
}

const EXPORT_HEADER: [&str; 10] = [
    "id",
    "時間(UTC)",
    "時間(店家當地)",
    "操作者類型",
    "操作者 ID",
    "操作者姓名",
    "動作",
    "對象類型",
    "對象 ID",
    "細節(JSON)",
];

/// 匯出稽核日誌(CSV,僅管理者以上)。篩選條件與列表相同。
///
/// 匯出本身也是敏感操作(一次帶走大量紀錄),所以會寫一筆 `audit.exported`(誰、篩選條件、筆數)。
/// 這個紀錄與資料讀取在同一個交易:匯出失敗(例如超過上限)就不會留下「匯出過」的假紀錄。
async fn export(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path(slug): Path<String>,
    Query(q): Query<ExportQuery>,
) -> Result<impl IntoResponse, AppError> {
    ctx.require_manager()?;
    if q.action.as_ref().is_some_and(|a| a.len() > 64) {
        return Err(AppError::BadRequest("action 過長".into()));
    }

    let mut tx = ctx.begin(&state).await?;
    let rows = sqlx::query_as::<_, ExportRow>(
        "SELECT a.id, a.created_at,
                to_char(a.created_at AT TIME ZONE t.timezone, 'YYYY-MM-DD HH24:MI:SS') AS created_local,
                a.actor_type, a.actor_user_id, u.name AS actor_name,
                a.action, a.entity_type, a.entity_id, a.detail
         FROM audit_logs a
         JOIN tenants t ON t.id = a.tenant_id
         LEFT JOIN users u ON u.id = a.actor_user_id
         WHERE ($1::text IS NULL OR left(a.action, length($1)) = $1)
           AND ($2::text IS NULL OR a.entity_type = $2)
           AND ($3::uuid IS NULL OR a.entity_id = $3)
           AND ($4::uuid IS NULL OR a.actor_user_id = $4)
           AND ($5::timestamptz IS NULL OR a.created_at >= $5)
           AND ($6::timestamptz IS NULL OR a.created_at < $6)
         ORDER BY a.id ASC LIMIT $7",
    )
    .bind(&q.action)
    .bind(&q.entity_type)
    .bind(q.entity_id)
    .bind(q.actor_user_id)
    .bind(q.from)
    .bind(q.to)
    .bind(EXPORT_MAX_ROWS + 1)
    .fetch_all(&mut *tx)
    .await?;
    if rows.len() as i64 > EXPORT_MAX_ROWS {
        return Err(AppError::BadRequest(format!(
            "符合條件的紀錄超過 {EXPORT_MAX_ROWS} 筆,請縮小日期範圍後再匯出"
        )));
    }

    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "audit.exported",
        "audit_log",
        None,
        serde_json::json!({
            "rows": rows.len(),
            "action": q.action,
            "entity_type": q.entity_type,
            "from": q.from,
            "to": q.to,
        }),
    )
    .await?;
    tx.commit().await?;

    let mut body = String::from(csv::BOM);
    body.push_str(&csv::row(EXPORT_HEADER));
    for r in &rows {
        body.push_str(&csv::row([
            r.id.to_string(),
            r.created_at
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            r.created_local.clone(),
            r.actor_type.clone(),
            r.actor_user_id.map(|u| u.to_string()).unwrap_or_default(),
            r.actor_name.clone().unwrap_or_default(),
            r.action.clone(),
            r.entity_type.clone(),
            r.entity_id.map(|u| u.to_string()).unwrap_or_default(),
            r.detail.to_string(),
        ]));
    }

    // 檔名只用店家代稱(只含 a-z 0-9 -)與日期,不會有需要跳脫的字元
    let filename = format!("audit-{slug}-{}.csv", Utc::now().format("%Y%m%d"));
    let headers: [(HeaderName, String); 3] = [
        (header::CONTENT_TYPE, "text/csv; charset=utf-8".into()),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        ),
        // 內含店家資料,不可被任何中間的快取留住
        (header::CACHE_CONTROL, "no-store".into()),
    ];
    Ok((StatusCode::OK, headers, body))
}
