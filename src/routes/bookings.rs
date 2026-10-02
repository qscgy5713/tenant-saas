//! 員工端預約管理(需登入)。owner / manager 看全部,staff 只看自己的。

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::AppState;
use crate::{
    booking::{self, BookingStatus, NewBooking},
    error::AppError,
    mail, outbox,
    tenancy::{Role, TenantCtx},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/t/{slug}/bookings", get(list).post(create))
        .route("/t/{slug}/bookings/{id}", patch(update))
}

#[derive(Debug, Serialize, FromRow)]
struct BookingItem {
    id: Uuid,
    status: BookingStatus,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    notes: Option<String>,
    service_id: Uuid,
    service_name: String,
    staff_id: Uuid,
    staff_name: String,
    customer_name: String,
    customer_email: String,
    customer_phone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    status: Option<BookingStatus>,
    staff_id: Option<Uuid>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Serialize)]
struct Page {
    items: Vec<BookingItem>,
    limit: i64,
    offset: i64,
}

async fn list(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Query(q): Query<ListQuery>,
) -> Result<Json<Page>, AppError> {
    // 員工只能看自己的預約
    let staff_filter = if ctx.role == Role::Staff {
        if q.staff_id.is_some_and(|id| id != ctx.user_id) {
            return Err(AppError::Forbidden);
        }
        Some(ctx.user_id)
    } else {
        q.staff_id
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 100);
    let offset = q.offset.unwrap_or(0).max(0);

    let mut tx = ctx.begin(&state).await?;
    let items = sqlx::query_as::<_, BookingItem>(
        "SELECT b.id, b.status, b.starts_at, b.ends_at, b.notes,
                s.id AS service_id, s.name AS service_name,
                u.id AS staff_id, u.name AS staff_name,
                c.name AS customer_name, c.email::text AS customer_email, c.phone AS customer_phone
         FROM bookings b
         JOIN services s ON s.id = b.service_id
         JOIN users u ON u.id = b.staff_user_id
         JOIN customers c ON c.id = b.customer_id
         WHERE ($1::timestamptz IS NULL OR b.ends_at > $1)
           AND ($2::timestamptz IS NULL OR b.starts_at < $2)
           AND ($3::booking_status IS NULL OR b.status = $3)
           AND ($4::uuid IS NULL OR b.staff_user_id = $4)
         ORDER BY b.starts_at, b.id LIMIT $5 OFFSET $6",
    )
    .bind(q.from)
    .bind(q.to)
    .bind(q.status)
    .bind(staff_filter)
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

#[derive(Debug, Serialize)]
struct Created {
    id: Uuid,
    staff_id: Uuid,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    manage_token: String,
}

/// 代客預約(電話、現場)。staff 只能替自己建立;時段驗證與顧客自助預約完全相同。
async fn create(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Json(mut req): Json<NewBooking>,
) -> Result<(StatusCode, Json<Created>), AppError> {
    if ctx.role == Role::Staff {
        match req.staff_id {
            Some(id) if id != ctx.user_id => return Err(AppError::Forbidden),
            _ => req.staff_id = Some(ctx.user_id),
        }
    }
    let mut tx = ctx.begin(&state).await?;
    let tz = booking::tenant_timezone(&mut tx, ctx.tenant_id).await?;
    let created =
        booking::create_booking(&mut tx, ctx.tenant_id, tz, req, BookingStatus::Confirmed).await?;
    let mail_ctx = booking::mail_ctx(&mut tx, created.id).await?;
    let link = mail::booking_link(&state.public_base_url, &created.token);
    let email = mail::confirmed(&mail_ctx.view(), &link);
    outbox::enqueue(
        &mut tx,
        ctx.tenant_id,
        &email,
        Some(&format!("confirmed:{}", created.id)),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(Created {
            id: created.id,
            staff_id: created.staff_id,
            starts_at: created.starts_at,
            ends_at: created.ends_at,
            manage_token: created.token,
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct UpdateBooking {
    status: Option<BookingStatus>,
    notes: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
struct Updated {
    id: Uuid,
    status: BookingStatus,
    notes: Option<String>,
}

async fn update(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, id)): Path<(String, Uuid)>,
    Json(req): Json<UpdateBooking>,
) -> Result<Json<Updated>, AppError> {
    if req.notes.as_ref().is_some_and(|n| n.chars().count() > 500) {
        return Err(AppError::BadRequest("備註最多 500 字".into()));
    }
    if matches!(
        req.status,
        Some(BookingStatus::Confirmed | BookingStatus::Pending)
    ) {
        return Err(AppError::BadRequest(
            "只能改為 cancelled、completed 或 no_show".into(),
        ));
    }

    let mut tx = ctx.begin(&state).await?;
    let row: Option<(Uuid, BookingStatus, DateTime<Utc>)> = sqlx::query_as(
        "SELECT staff_user_id, status, starts_at FROM bookings WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (staff_id, status, starts_at) = row.ok_or(AppError::NotFound)?;
    if ctx.role == Role::Staff && staff_id != ctx.user_id {
        return Err(AppError::Forbidden);
    }

    if let Some(new_status) = req.status {
        // 取消、完成、未到都是終態:已取消的時段可能已被別人訂走,不能復活
        if status != BookingStatus::Confirmed {
            return Err(AppError::Conflict("此預約已取消或已結束".into()));
        }
        if matches!(new_status, BookingStatus::Completed | BookingStatus::NoShow)
            && starts_at > Utc::now()
        {
            return Err(AppError::BadRequest(
                "預約尚未開始,不能標記為完成或未到".into(),
            ));
        }
    }

    let updated = sqlx::query_as::<_, Updated>(
        "UPDATE bookings SET status = COALESCE($2, status), notes = COALESCE($3, notes)
         WHERE id = $1 RETURNING id, status, notes",
    )
    .bind(id)
    .bind(req.status)
    .bind(req.notes)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(updated))
}
