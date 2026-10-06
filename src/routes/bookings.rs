//! 員工端預約管理(需登入)。owner / manager 看全部,staff 只看自己的。

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderName, StatusCode, header},
    response::IntoResponse,
    routing::{get, patch, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use serde_json::json;

use super::{
    AppState,
    public::{AvailabilityResponse, RangeQuery, validated_range},
};
use crate::{
    audit::{self, Actor},
    booking::{self, BookingStatus, NewBooking},
    csv::{self, EXPORT_MAX_ROWS},
    error::AppError,
    mail, outbox,
    tenancy::{Role, TenantCtx},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/t/{slug}/bookings", get(list).post(create))
        .route("/t/{slug}/bookings/export.csv", get(export))
        .route("/t/{slug}/bookings/{id}", patch(update))
        .route("/t/{slug}/bookings/{id}/availability", get(availability))
        .route("/t/{slug}/bookings/{id}/reschedule", post(reschedule))
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
    outbox::ensure_recipient_quota(&mut tx, &email.to, state.max_mails_per_recipient_per_hour)
        .await?;
    outbox::enqueue(
        &mut tx,
        ctx.tenant_id,
        &email,
        Some(&format!("confirmed:{}", created.id)),
    )
    .await?;
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "booking.created",
        "booking",
        Some(created.id),
        json!({ "source": "staff", "staff_id": created.staff_id, "starts_at": created.starts_at }),
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
        if !matches!(status, BookingStatus::Confirmed | BookingStatus::Pending) {
            return Err(AppError::Conflict("此預約已取消或已結束".into()));
        }
        // 完成、未到只適用於已確認且已開始的預約;待確認的預約只能取消
        if matches!(new_status, BookingStatus::Completed | BookingStatus::NoShow) {
            if status == BookingStatus::Pending {
                return Err(AppError::BadRequest(
                    "尚未確認的預約不能標記為完成或未到".into(),
                ));
            }
            if starts_at > Utc::now() {
                return Err(AppError::BadRequest(
                    "預約尚未開始,不能標記為完成或未到".into(),
                ));
            }
        }
    }

    let notes_changed = req.notes.is_some();
    let updated = sqlx::query_as::<_, Updated>(
        "UPDATE bookings SET status = COALESCE($2, status), notes = COALESCE($3, notes)
         WHERE id = $1 RETURNING id, status, notes",
    )
    .bind(id)
    .bind(req.status)
    .bind(req.notes)
    .fetch_one(&mut *tx)
    .await?;
    // 備註可能含顧客個資,稽核只記「備註有異動」,不記內容
    let (action, detail) = match req.status {
        Some(to) => (
            match to {
                BookingStatus::Cancelled => "booking.cancelled",
                BookingStatus::Completed => "booking.completed",
                _ => "booking.no_show",
            },
            json!({ "from": status, "to": to, "notes_changed": notes_changed }),
        ),
        None => ("booking.notes_updated", json!({ "notes_changed": true })),
    };
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        action,
        "booking",
        Some(id),
        detail,
    )
    .await?;

    // 取消「已確認、還沒開始」的預約要通知:顧客一定要知道;若是管理者取消別人負責的預約,也通知那位員工。
    // 待確認的不寄(顧客的 Email 還沒驗證過);已經開始 / 過去的預約不寄(通知「取消」只會造成困惑)
    if req.status == Some(BookingStatus::Cancelled)
        && status == BookingStatus::Confirmed
        && starts_at > Utc::now()
    {
        let m = booking::mail_ctx(&mut tx, id).await?;
        let link = mail::shop_page_link(&state.public_base_url, &m.slug);
        outbox::enqueue(
            &mut tx,
            ctx.tenant_id,
            &mail::cancelled_by_shop(&m.view(), &link),
            Some(&format!("cancelled:{id}:customer")),
        )
        .await?;
        if staff_id != ctx.user_id {
            let email = mail::cancelled_for_staff(
                &m.view(),
                &m.staff_email,
                &m.customer_name,
                mail::CancelledBy::Manager,
            );
            outbox::enqueue(
                &mut tx,
                ctx.tenant_id,
                &email,
                Some(&format!("cancelled:{id}:staff")),
            )
            .await?;
        }
    }
    tx.commit().await?;
    Ok(Json(updated))
}

/// 員工對這筆預約能不能操作:管理者以上,或自己的預約
fn require_own_or_manager(ctx: &TenantCtx, staff_id: Uuid) -> Result<(), AppError> {
    if ctx.role == Role::Staff && staff_id != ctx.user_id {
        Err(AppError::Forbidden)
    } else {
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct Target {
    staff_user_id: Uuid,
    service_id: Uuid,
    status: BookingStatus,
    starts_at: DateTime<Utc>,
}

async fn load_target(tx: &mut crate::db::Tx, id: Uuid, lock: bool) -> Result<Target, AppError> {
    // 兩條固定的 SQL(sqlx 不接受動態拼接的字串)
    let query = if lock {
        "SELECT staff_user_id, service_id, status, starts_at FROM bookings WHERE id = $1 FOR UPDATE"
    } else {
        "SELECT staff_user_id, service_id, status, starts_at FROM bookings WHERE id = $1"
    };
    sqlx::query_as::<_, Target>(query)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(AppError::NotFound)
}

/// 這筆預約改期時可選的時段(同一位員工與服務,排除這筆預約自己)
async fn availability(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, id)): Path<(String, Uuid)>,
    Query(q): Query<RangeQuery>,
) -> Result<Json<AvailabilityResponse>, AppError> {
    let to = validated_range(q.from, q.to)?;
    let mut tx = ctx.begin(&state).await?;
    let target = load_target(&mut tx, id, false).await?;
    require_own_or_manager(&ctx, target.staff_user_id)?;
    if target.status != BookingStatus::Confirmed {
        return Err(AppError::Conflict("只有已確認的預約可以改期".into()));
    }
    let tz = booking::tenant_timezone(&mut tx, ctx.tenant_id).await?;
    let service = booking::active_service(&mut tx, target.service_id).await?;
    let slots = booking::availability(
        &mut tx,
        tz,
        &service,
        q.from,
        to,
        Some(target.staff_user_id),
        Some(id),
    )
    .await?;
    tx.rollback().await?;
    Ok(Json(AvailabilityResponse {
        timezone: tz.name().to_string(),
        slots,
    }))
}

#[derive(Debug, Deserialize)]
struct RescheduleBody {
    start: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct Rescheduled {
    id: Uuid,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
}

/// 員工替顧客改期。規則與顧客自己改期相同(同一位員工、同一個服務、跨月要算額度),
/// 差別是:額度不足回 402(員工看得到方案資訊),並寄通知信告訴顧客時間變了。
async fn reschedule(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, id)): Path<(String, Uuid)>,
    Json(req): Json<RescheduleBody>,
) -> Result<Json<Rescheduled>, AppError> {
    let mut tx = ctx.begin(&state).await?;
    let target = load_target(&mut tx, id, true).await?;
    require_own_or_manager(&ctx, target.staff_user_id)?;
    if target.status != BookingStatus::Confirmed {
        return Err(AppError::Conflict("只有已確認的預約可以改期".into()));
    }
    if target.starts_at <= Utc::now() {
        return Err(AppError::Conflict("預約已開始,無法改期".into()));
    }
    let tz = booking::tenant_timezone(&mut tx, ctx.tenant_id).await?;
    let service = booking::active_service(&mut tx, target.service_id).await?;
    let old_start = booking::reschedule_booking(
        &mut tx,
        ctx.tenant_id,
        tz,
        id,
        &service,
        target.staff_user_id,
        req.start,
    )
    .await?;

    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "booking.rescheduled",
        "booking",
        Some(id),
        json!({ "source": "staff", "from_starts_at": old_start, "to_starts_at": req.start }),
    )
    .await?;

    let mail_ctx = booking::mail_ctx(&mut tx, id).await?;
    let email = mail::rescheduled(&mail_ctx.view(), &mail::format_local(old_start, tz));
    outbox::enqueue(
        &mut tx,
        ctx.tenant_id,
        &email,
        Some(&format!("rescheduled:{id}:{}", req.start.timestamp())),
    )
    .await?;

    let ends_at = req.start + service.duration();
    tx.commit().await?;
    Ok(Json(Rescheduled {
        id,
        starts_at: req.start,
        ends_at,
    }))
}

#[derive(Debug, Deserialize)]
struct ExportQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    status: Option<BookingStatus>,
    staff_id: Option<Uuid>,
}

#[derive(Debug, FromRow)]
struct ExportRow {
    id: Uuid,
    status: BookingStatus,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    starts_local: String,
    ends_local: String,
    service_name: String,
    staff_name: String,
    customer_name: String,
    customer_email: String,
    customer_phone: Option<String>,
    notes: Option<String>,
}

const EXPORT_HEADER: [&str; 12] = [
    "預約編號",
    "狀態",
    "開始(UTC)",
    "開始(店家當地)",
    "結束(UTC)",
    "結束(店家當地)",
    "服務",
    "服務人員",
    "顧客姓名",
    "顧客 Email",
    "顧客電話",
    "備註",
];

fn status_label(status: BookingStatus) -> &'static str {
    match status {
        BookingStatus::Pending => "待確認",
        BookingStatus::Confirmed => "已確認",
        BookingStatus::Completed => "已完成",
        BookingStatus::NoShow => "未到",
        BookingStatus::Cancelled => "已取消",
    }
}

/// 匯出預約(CSV,僅管理者以上;員工不能一次帶走全店的顧客資料)。
///
/// 內含顧客的姓名 / Email / 電話與備註,所以:套用與稽核匯出相同的公式注入防護、
/// 超過上限就拒絕(不截斷)、匯出本身留一筆 `booking.exported`(誰、條件、幾筆;不含任何個資)。
/// 篩選的語意與列表相同:`from` / `to` 是「與這段時間有重疊的預約」。
async fn export(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path(slug): Path<String>,
    Query(q): Query<ExportQuery>,
) -> Result<impl IntoResponse, AppError> {
    ctx.require_manager()?;
    let mut tx = ctx.begin(&state).await?;
    let rows = sqlx::query_as::<_, ExportRow>(
        "SELECT b.id, b.status, b.starts_at, b.ends_at,
                to_char(b.starts_at AT TIME ZONE t.timezone, 'YYYY-MM-DD HH24:MI') AS starts_local,
                to_char(b.ends_at AT TIME ZONE t.timezone, 'YYYY-MM-DD HH24:MI') AS ends_local,
                s.name AS service_name, u.name AS staff_name,
                c.name AS customer_name, c.email::text AS customer_email, c.phone AS customer_phone,
                b.notes
         FROM bookings b
         JOIN tenants t ON t.id = b.tenant_id
         JOIN services s ON s.id = b.service_id
         JOIN users u ON u.id = b.staff_user_id
         JOIN customers c ON c.id = b.customer_id
         WHERE ($1::timestamptz IS NULL OR b.ends_at > $1)
           AND ($2::timestamptz IS NULL OR b.starts_at < $2)
           AND ($3::booking_status IS NULL OR b.status = $3)
           AND ($4::uuid IS NULL OR b.staff_user_id = $4)
         ORDER BY b.starts_at, b.id LIMIT $5",
    )
    .bind(q.from)
    .bind(q.to)
    .bind(q.status)
    .bind(q.staff_id)
    .bind(EXPORT_MAX_ROWS + 1)
    .fetch_all(&mut *tx)
    .await?;
    if rows.len() as i64 > EXPORT_MAX_ROWS {
        return Err(AppError::BadRequest(format!(
            "符合條件的預約超過 {EXPORT_MAX_ROWS} 筆,請縮小日期範圍後再匯出"
        )));
    }

    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "booking.exported",
        "booking",
        None,
        json!({ "rows": rows.len(), "status": q.status, "from": q.from, "to": q.to }),
    )
    .await?;
    tx.commit().await?;

    let mut body = String::from(csv::BOM);
    body.push_str(&csv::row(EXPORT_HEADER));
    for r in &rows {
        body.push_str(&csv::row([
            r.id.to_string(),
            status_label(r.status).to_string(),
            r.starts_at
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            r.starts_local.clone(),
            r.ends_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            r.ends_local.clone(),
            r.service_name.clone(),
            r.staff_name.clone(),
            r.customer_name.clone(),
            r.customer_email.clone(),
            r.customer_phone.clone().unwrap_or_default(),
            r.notes.clone().unwrap_or_default(),
        ]));
    }
    let filename = format!("bookings-{slug}-{}.csv", Utc::now().format("%Y%m%d"));
    let headers: [(HeaderName, String); 3] = [
        (header::CONTENT_TYPE, "text/csv; charset=utf-8".into()),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        ),
        (header::CACHE_CONTROL, "no-store".into()),
    ];
    Ok((StatusCode::OK, headers, body))
}
