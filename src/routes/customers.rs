use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::AppState;
use crate::{
    audit::{self, Actor},
    error::AppError,
    tenancy::TenantCtx,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/t/{slug}/customers", get(list))
        .route("/t/{slug}/customers/{id}", get(detail))
        .route("/t/{slug}/customers/{id}/anonymize", post(anonymize))
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
    // 瀏覽顧客個資要留紀錄(誰、何時、看了幾筆)。**不記搜尋字串**:搜尋的常常就是姓名 / 電話 / Email,
    // 稽核日誌只能新增、不能刪,寫進去的個資就收不回來了
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "customer.listed",
        "customer",
        None,
        serde_json::json!({ "count": items.len(), "offset": offset, "searched": pattern.is_some() }),
    )
    .await?;
    tx.commit().await?;
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
    // 看了這位顧客的完整資料與預約紀錄(找不到的不會走到這裡,所以不會留下紀錄)
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "customer.viewed",
        "customer",
        Some(id),
        serde_json::json!({ "history": history.len() }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(Detail { customer, history }))
}

/// 匿名化之後的顧客 Email 一律以這個網域結尾(`.invalid` 是保留的頂級網域,保證不是真的地址)
const ANON_DOMAIN: &str = "@anonymized.invalid";
const ANON_NAME: &str = "已刪除的顧客";

/// 刪除顧客個資(顧客要求刪除自己的資料時用)。**不可還原,只有擁有者能做。**
///
/// 不是刪除預約:預約有統計、員工行程與稽核的關聯,而且取消已經涵蓋「這筆不要了」。
/// 這裡把「這位顧客是誰」抹掉 —— 姓名、Email、電話換成無法還原的代號,預約上的備註清掉
/// (備註常常寫著個資),還沒寄出的信刪掉、已寄出的信件紀錄改成匿名地址 —— 預約本身保留。
///
/// 還有未來的已確認預約或待確認的申請時不能做:那位顧客還等著被服務、還會收到信。先取消再刪。
/// 沒有涵蓋:日誌與備份(依你們的保留政策處理)。稽核日誌本來就不存個資。
async fn anonymize(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, id)): Path<(String, Uuid)>,
) -> Result<StatusCode, AppError> {
    ctx.require_owner()?;
    let mut tx = ctx.begin(&state).await?;
    let email: String =
        sqlx::query_scalar("SELECT email::text FROM customers WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;
    if email.ends_with(ANON_DOMAIN) {
        return Err(AppError::Conflict("這位顧客的資料已經刪除過了".into()));
    }
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM bookings WHERE customer_id = $1
           AND (status = 'pending' OR (status = 'confirmed' AND starts_at > now()))",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if live > 0 {
        return Err(AppError::Conflict(format!(
            "這位顧客還有 {live} 筆未來或待確認的預約,請先取消再刪除資料"
        )));
    }

    let anon_email = format!("deleted-{id}{ANON_DOMAIN}");
    sqlx::query("UPDATE customers SET name = $2, email = $3, phone = NULL WHERE id = $1")
        .bind(id)
        .bind(ANON_NAME)
        .bind(&anon_email)
        .execute(&mut *tx)
        .await?;
    let bookings = sqlx::query("UPDATE bookings SET notes = NULL WHERE customer_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let unsent: i32 = sqlx::query_scalar("SELECT outbox_forget_recipient($1, $2)")
        .bind(&email)
        .bind(&anon_email)
        .fetch_one(&mut *tx)
        .await?;
    // 稽核只記「刪了誰(id)、影響幾筆」,不記任何個資
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "customer.anonymized",
        "customer",
        Some(id),
        serde_json::json!({ "bookings": bookings, "unsent_mails_removed": unsent }),
    )
    .await?;
    tx.commit().await?;
    // 資料庫以外的紀錄(只有 id):從備份還原會把已刪除的資料救回來,要靠它知道哪些得重做。見 docs/deployment.md
    tracing::info!(customer_id = %id, tenant_id = %ctx.tenant_id, "顧客資料已刪除(匿名化)");
    Ok(StatusCode::NO_CONTENT)
}
