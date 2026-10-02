use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::AppState;
use crate::{error::AppError, tenancy::TenantCtx};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/t/{slug}/customers", get(list))
        .route("/t/{slug}/customers/{id}", get(detail))
}

/// 顧客清單只列「確認過」的顧客(SQL 內的 status IN ('confirmed', 'completed', 'no_show')):至少有一筆已確認 / 已完成 / 未到的預約。
/// 只有待確認的預約不算 —— 那只是有人輸入了某個 Email,沒有證據這個 Email 是本人的。

#[derive(Debug, Deserialize)]
struct ListQuery {
    q: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
struct CustomerItem {
    id: Uuid,
    name: String,
    email: String,
    phone: Option<String>,
    /// 已確認 + 已完成 + 未到
    bookings: i64,
    completed: i64,
    no_shows: i64,
    /// 最近一次(已開始)的預約
    last_at: Option<DateTime<Utc>>,
    /// 下一次(尚未開始、已確認)的預約
    next_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct Page {
    items: Vec<CustomerItem>,
    limit: i64,
    offset: i64,
}

/// LIKE 的萬用字元要跳脫,否則搜尋 `%` 會列出全部
fn like_pattern(q: &str) -> String {
    let mut out = String::from("%");
    for c in q.trim().chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

async fn list(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path(_slug): Path<String>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Page>, AppError> {
    // 顧客的聯絡資料只給管理者以上(員工從自己的預約就能看到需要的部分)
    ctx.require_manager()?;
    let limit = q.limit.unwrap_or(50).clamp(1, 100);
    let offset = q.offset.unwrap_or(0).max(0);
    let pattern =
        q.q.as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(like_pattern);
    if pattern.as_ref().is_some_and(|p| p.chars().count() > 102) {
        return Err(AppError::BadRequest("搜尋字串過長".into()));
    }

    let mut tx = ctx.begin(&state).await?;
    let items = sqlx::query_as::<_, CustomerItem>(
        "SELECT c.id, c.name, c.email::text AS email, c.phone,
                count(*) AS bookings,
                count(*) FILTER (WHERE b.status = 'completed') AS completed,
                count(*) FILTER (WHERE b.status = 'no_show') AS no_shows,
                max(b.starts_at) FILTER (WHERE b.starts_at <= now()) AS last_at,
                min(b.starts_at) FILTER (WHERE b.starts_at > now() AND b.status = 'confirmed') AS next_at
         FROM customers c JOIN bookings b ON b.customer_id = c.id AND b.status IN ('confirmed', 'completed', 'no_show')
         WHERE ($1::text IS NULL OR c.name ILIKE $1 OR c.email::text ILIKE $1 OR c.phone ILIKE $1)
         GROUP BY c.id
         ORDER BY max(b.starts_at) DESC, c.id
         LIMIT $2 OFFSET $3",
    )
    .bind(&pattern)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    Ok(Json(Page {
        items,
        limit,
        offset,
    }))
}

#[derive(Debug, Serialize, FromRow)]
struct History {
    id: Uuid,
    status: String,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    notes: Option<String>,
    service_name: String,
    staff_name: String,
}

#[derive(Debug, Serialize)]
struct Detail {
    #[serde(flatten)]
    customer: CustomerItem,
    history: Vec<History>,
}

async fn detail(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, id)): Path<(String, Uuid)>,
) -> Result<Json<Detail>, AppError> {
    ctx.require_manager()?;
    let mut tx = ctx.begin(&state).await?;
    let customer = sqlx::query_as::<_, CustomerItem>(
        "SELECT c.id, c.name, c.email::text AS email, c.phone,
                count(*) AS bookings,
                count(*) FILTER (WHERE b.status = 'completed') AS completed,
                count(*) FILTER (WHERE b.status = 'no_show') AS no_shows,
                max(b.starts_at) FILTER (WHERE b.starts_at <= now()) AS last_at,
                min(b.starts_at) FILTER (WHERE b.starts_at > now() AND b.status = 'confirmed') AS next_at
         FROM customers c JOIN bookings b ON b.customer_id = c.id AND b.status IN ('confirmed', 'completed', 'no_show')
         WHERE c.id = $1
         GROUP BY c.id",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    // 歷史包含已取消的,但不含沒人確認過的待確認申請
    let history = sqlx::query_as::<_, History>(
        "SELECT b.id, b.status::text AS status, b.starts_at, b.ends_at, b.notes,
                s.name AS service_name, u.name AS staff_name
         FROM bookings b
         JOIN services s ON s.id = b.service_id
         JOIN users u ON u.id = b.staff_user_id
         WHERE b.customer_id = $1 AND b.status <> 'pending'
         ORDER BY b.starts_at DESC LIMIT 100",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    Ok(Json(Detail { customer, history }))
}
