//! 公開預約頁:顧客不需帳號。
//!
//! 租戶由網址代稱或預約連結 token 決定,解析後照一般流程設定租戶上下文,仍受 RLS 約束。
//! 這裡的 handler 只回傳公開資訊(服務、員工姓名、可預約時段),
//! 絕不回傳其他顧客的資料。

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sqlx::{Acquire, FromRow};
use uuid::Uuid;

use serde_json::json;

use super::AppState;
use crate::{
    audit::{self, Actor},
    availability::Slot,
    booking::{self, BookingStatus, NewBooking, PENDING_TTL_HOURS},
    db::{PG_EXCLUSION_VIOLATION, Tx, begin_scoped, pg_code, set_tenant},
    error::AppError,
    mail, outbox, plan, token,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/public/shops/{slug}", get(shop))
        .route("/public/shops/{slug}/services", get(services))
        .route(
            "/public/shops/{slug}/services/{service_id}/staff",
            get(service_staff),
        )
        .route("/public/shops/{slug}/availability", get(availability))
        .route("/public/shops/{slug}/bookings", post(create_booking))
        .route("/public/bookings/{token}", get(get_booking))
        .route("/public/bookings/{token}/confirm", post(confirm_booking))
        .route("/public/bookings/{token}/cancel", post(cancel_booking))
        .route(
            "/public/bookings/{token}/reschedule",
            post(reschedule_booking),
        )
}

/// 對顧客不暴露店家的方案與限額:額滿一律說成「該月份已額滿」(顧客也無法升級方案)
fn hide_plan_details(err: AppError) -> AppError {
    match err {
        AppError::LimitReached(_) => {
            AppError::Conflict("這家店該月份的預約已額滿,請選擇其他月份或直接聯絡店家".into())
        }
        other => other,
    }
}

/// 系統代為取消(確認當下發現無法成立)。理由寫進稽核,方便事後解釋「為什麼我的預約不見了」。
async fn auto_cancel(
    tx: &mut Tx,
    tenant_id: Uuid,
    booking_id: Uuid,
    reason: &str,
) -> Result<(), AppError> {
    sqlx::query("UPDATE bookings SET status = 'cancelled' WHERE id = $1")
        .bind(booking_id)
        .execute(&mut **tx)
        .await?;
    audit::record(
        tx,
        tenant_id,
        Actor::System,
        "booking.cancelled",
        "booking",
        Some(booking_id),
        json!({ "to": BookingStatus::Cancelled, "reason": reason }),
    )
    .await
}

/// 依網址代稱開啟已設定租戶上下文的交易。店家不存在或已停權一律 404。
async fn open_shop(state: &AppState, slug: &str) -> Result<(Tx, Uuid, Tz), AppError> {
    let mut tx = begin_scoped(&state.db, None, None).await?;
    let tenant_id: Option<Uuid> = sqlx::query_scalar("SELECT tenant_id_by_slug($1)")
        .bind(slug)
        .fetch_one(&mut *tx)
        .await?;
    let tenant_id = tenant_id.ok_or(AppError::NotFound)?;
    set_tenant(&mut tx, tenant_id).await?;
    let tz = booking::tenant_timezone(&mut tx, tenant_id).await?;
    Ok((tx, tenant_id, tz))
}

/// 依預約連結 token 開啟交易。token 無效一律 404。
async fn open_by_token(state: &AppState, raw: &str) -> Result<(Tx, Uuid, Tz), AppError> {
    if !(16..=128).contains(&raw.len()) {
        return Err(AppError::NotFound);
    }
    let mut tx = begin_scoped(&state.db, None, None).await?;
    let tenant_id: Option<Uuid> = sqlx::query_scalar("SELECT booking_tenant_by_token($1)")
        .bind(token::hash(raw))
        .fetch_one(&mut *tx)
        .await?;
    let tenant_id = tenant_id.ok_or(AppError::NotFound)?;
    set_tenant(&mut tx, tenant_id).await?;
    let tz = booking::tenant_timezone(&mut tx, tenant_id).await?;
    Ok((tx, tenant_id, tz))
}

// ---------- 店家與服務 ----------

#[derive(Debug, Serialize)]
struct Shop {
    name: String,
    timezone: String,
}

async fn shop(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Shop>, AppError> {
    let (mut tx, tenant_id, _) = open_shop(&state, &slug).await?;
    let (name, timezone): (String, String) =
        sqlx::query_as("SELECT name, timezone FROM tenants WHERE id = $1")
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;
    tx.rollback().await?;
    Ok(Json(Shop { name, timezone }))
}

#[derive(Debug, Serialize, FromRow)]
struct PublicService {
    id: Uuid,
    name: String,
    duration_minutes: i32,
    price_cents: i32,
}

async fn services(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<PublicService>>, AppError> {
    let (mut tx, _, _) = open_shop(&state, &slug).await?;
    let rows = sqlx::query_as::<_, PublicService>(
        "SELECT id, name, duration_minutes, price_cents FROM services
         WHERE active ORDER BY name, id LIMIT 200",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    Ok(Json(rows))
}

#[derive(Debug, Serialize, FromRow)]
struct PublicStaff {
    id: Uuid,
    name: String,
}

async fn service_staff(
    State(state): State<AppState>,
    Path((slug, service_id)): Path<(String, Uuid)>,
) -> Result<Json<Vec<PublicStaff>>, AppError> {
    let (mut tx, _, _) = open_shop(&state, &slug).await?;
    booking::active_service(&mut tx, service_id).await?;
    let rows = sqlx::query_as::<_, PublicStaff>(
        "SELECT u.id, u.name FROM staff_services ss JOIN users u ON u.id = ss.user_id
         WHERE ss.service_id = $1 ORDER BY u.name, u.id",
    )
    .bind(service_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    Ok(Json(rows))
}

// ---------- 可預約時段 ----------

const MAX_RANGE_DAYS: i64 = 14;

#[derive(Debug, Deserialize)]
struct AvailabilityQuery {
    service_id: Uuid,
    /// 店家本地日期 YYYY-MM-DD
    from: NaiveDate,
    to: Option<NaiveDate>,
    staff_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct AvailabilityResponse {
    timezone: String,
    slots: Vec<Slot>,
}

async fn availability(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<AvailabilityQuery>,
) -> Result<Json<AvailabilityResponse>, AppError> {
    let to = q.to.unwrap_or(q.from);
    if to < q.from {
        return Err(AppError::BadRequest("結束日期不可早於開始日期".into()));
    }
    if (to - q.from).num_days() >= MAX_RANGE_DAYS {
        return Err(AppError::BadRequest(format!(
            "一次最多查詢 {MAX_RANGE_DAYS} 天"
        )));
    }
    let (mut tx, _, tz) = open_shop(&state, &slug).await?;
    let service = booking::active_service(&mut tx, q.service_id).await?;
    let slots = booking::availability(&mut tx, tz, &service, q.from, to, q.staff_id, None).await?;
    tx.rollback().await?;
    Ok(Json(AvailabilityResponse {
        timezone: tz.name().to_string(),
        slots,
    }))
}

// ---------- 預約 ----------

#[derive(Debug, Serialize)]
struct BookingRequested {
    id: Uuid,
    status: BookingStatus,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    message: &'static str,
}

/// 顧客自助預約:建立「待確認」的預約並寄確認信。
/// 時段在對方按下確認之前不會被占住,所以即使有人冒用別人的 Email 預約,也占不到任何時段。
/// 回應刻意不含管理 token:token 只寄到該 Email,拿不到信就無法替對方確認。
async fn create_booking(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<NewBooking>,
) -> Result<(StatusCode, Json<BookingRequested>), AppError> {
    let (mut tx, tenant_id, tz) = open_shop(&state, &slug).await?;
    let created = booking::create_booking(&mut tx, tenant_id, tz, req, BookingStatus::Pending)
        .await
        .map_err(hide_plan_details)?;

    let ctx = booking::mail_ctx(&mut tx, created.id).await?;
    let link = mail::booking_link(&state.public_base_url, &created.token);
    let email = mail::verification(&ctx.view(), &link);
    outbox::enqueue(
        &mut tx,
        tenant_id,
        &email,
        Some(&format!("verify:{}", created.id)),
    )
    .await?;
    audit::record(
        &mut tx,
        tenant_id,
        Actor::Customer,
        "booking.requested",
        "booking",
        Some(created.id),
        json!({ "staff_id": created.staff_id, "starts_at": created.starts_at }),
    )
    .await?;
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(BookingRequested {
            id: created.id,
            status: BookingStatus::Pending,
            starts_at: created.starts_at,
            ends_at: created.ends_at,
            message: "請到信箱按下確認連結,預約才會成立",
        }),
    ))
}

#[derive(Debug, Serialize, FromRow)]
struct PublicBooking {
    id: Uuid,
    status: BookingStatus,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    shop_name: String,
    /// 店家網址代稱與服務 / 員工 id:前端改期時要用它們查可預約時段(都不是機密)
    shop_slug: String,
    timezone: String,
    service_id: Uuid,
    service_name: String,
    staff_id: Uuid,
    staff_name: String,
    customer_name: String,
}

async fn load_by_token(tx: &mut Tx, raw: &str) -> Result<PublicBooking, AppError> {
    sqlx::query_as::<_, PublicBooking>(
        "SELECT b.id, b.status, b.starts_at, b.ends_at,
                t.name AS shop_name, t.slug AS shop_slug, t.timezone,
                b.service_id, s.name AS service_name,
                b.staff_user_id AS staff_id, u.name AS staff_name, c.name AS customer_name
         FROM bookings b
         JOIN tenants t ON t.id = b.tenant_id
         JOIN services s ON s.id = b.service_id
         JOIN users u ON u.id = b.staff_user_id
         JOIN customers c ON c.id = b.customer_id
         WHERE b.manage_token_hash = $1",
    )
    .bind(token::hash(raw))
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(AppError::NotFound)
}

async fn get_booking(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<Json<PublicBooking>, AppError> {
    let (mut tx, _, _) = open_by_token(&state, &raw).await?;
    let view = load_by_token(&mut tx, &raw).await?;
    tx.rollback().await?;
    Ok(Json(view))
}

#[derive(FromRow)]
struct PendingRow {
    id: Uuid,
    status: BookingStatus,
    starts_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    service_id: Uuid,
    staff_id: Uuid,
}

/// 顧客按下信中的確認連結後呼叫(前端頁面讓使用者按按鈕才送出,不用 GET,避免信件掃描器自動點開就確認)。
/// 重複確認視為成功(使用者雙擊或重新整理)。
async fn confirm_booking(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<Json<PublicBooking>, AppError> {
    let (mut tx, tenant_id, tz) = open_by_token(&state, &raw).await?;
    let PendingRow {
        id,
        status,
        starts_at,
        created_at,
        service_id,
        staff_id,
    } = sqlx::query_as::<_, PendingRow>(
        "SELECT id, status, starts_at, created_at, service_id, staff_user_id AS staff_id
         FROM bookings WHERE manage_token_hash = $1 FOR UPDATE",
    )
    .bind(token::hash(&raw))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;

    match status {
        BookingStatus::Confirmed => {
            let view = load_by_token(&mut tx, &raw).await?;
            tx.rollback().await?;
            return Ok(Json(view));
        }
        BookingStatus::Pending => {}
        _ => return Err(AppError::Conflict("此預約已取消或已結束".into())),
    }
    if created_at < Utc::now() - Duration::hours(i64::from(PENDING_TTL_HOURS))
        || starts_at <= Utc::now()
    {
        return Err(AppError::Conflict("確認連結已過期,請重新預約".into()));
    }

    // 申請之後營業時間、休假、服務狀態可能變了:確認的這一刻重新檢查
    if !booking::still_bookable(&mut tx, tz, id, service_id, staff_id, starts_at).await? {
        auto_cancel(&mut tx, tenant_id, id, "no_longer_bookable").await?;
        tx.commit().await?;
        return Err(AppError::Conflict(
            "這個時段已無法預約,請重新選擇時間".into(),
        ));
    }

    // 確認就是占位,要計入每月額度:序列化檢查。額滿就把這筆標為取消並明確告知(而不是讓連結一直掛著)
    if let Err(err) = plan::ensure_booking_slot(&mut tx, tenant_id, tz, starts_at, true).await {
        if matches!(err, AppError::LimitReached(_)) {
            auto_cancel(&mut tx, tenant_id, id, "monthly_limit").await?;
            tx.commit().await?;
            return Err(AppError::Conflict(
                "這家店本月預約已額滿,暫時無法接受新預約".into(),
            ));
        }
        return Err(err);
    }

    // 這一刻才真正占住時段,由排除約束把關;用 savepoint 才能在衝突後繼續更新狀態
    let mut sp = tx.begin().await?;
    let result =
        sqlx::query("UPDATE bookings SET status = 'confirmed', confirmed_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *sp)
            .await;
    match result {
        Ok(_) => sp.commit().await?,
        Err(e) if pg_code(&e).as_deref() == Some(PG_EXCLUSION_VIOLATION) => {
            sp.rollback().await?;
            auto_cancel(&mut tx, tenant_id, id, "slot_taken").await?;
            tx.commit().await?;
            return Err(AppError::Conflict(
                "這個時段在您確認之前已被其他人預約,請重新選擇時間".into(),
            ));
        }
        Err(e) => return Err(e.into()),
    }

    let ctx = booking::mail_ctx(&mut tx, id).await?;
    let link = mail::booking_link(&state.public_base_url, &raw);
    let email = mail::confirmed(&ctx.view(), &link);
    outbox::enqueue(&mut tx, tenant_id, &email, Some(&format!("confirmed:{id}"))).await?;
    audit::record(
        &mut tx,
        tenant_id,
        Actor::Customer,
        "booking.confirmed",
        "booking",
        Some(id),
        audit::empty(),
    )
    .await?;
    let view = load_by_token(&mut tx, &raw).await?;
    tx.commit().await?;
    Ok(Json(view))
}

async fn cancel_booking(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<Json<PublicBooking>, AppError> {
    let (mut tx, tenant_id, _) = open_by_token(&state, &raw).await?;
    let (id, _, _, from) = lock_changeable(&mut tx, &raw, true).await?;
    sqlx::query("UPDATE bookings SET status = 'cancelled' WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        tenant_id,
        Actor::Customer,
        "booking.cancelled",
        "booking",
        Some(id),
        json!({ "from": from, "to": BookingStatus::Cancelled }),
    )
    .await?;
    let view = load_by_token(&mut tx, &raw).await?;
    tx.commit().await?;
    Ok(Json(view))
}

#[derive(Debug, Deserialize)]
struct RescheduleRequest {
    start: DateTime<Utc>,
}

async fn reschedule_booking(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    Json(req): Json<RescheduleRequest>,
) -> Result<Json<PublicBooking>, AppError> {
    let (mut tx, tenant_id, tz) = open_by_token(&state, &raw).await?;
    let (id, service_id, staff_id, _) = lock_changeable(&mut tx, &raw, false).await?;
    let service = booking::active_service(&mut tx, service_id).await?;
    let old_start =
        booking::reschedule_booking(&mut tx, tenant_id, tz, id, &service, staff_id, req.start)
            .await
            .map_err(hide_plan_details)?;
    audit::record(
        &mut tx,
        tenant_id,
        Actor::Customer,
        "booking.rescheduled",
        "booking",
        Some(id),
        json!({ "from_starts_at": old_start, "to_starts_at": req.start }),
    )
    .await?;
    let view = load_by_token(&mut tx, &raw).await?;
    tx.commit().await?;
    Ok(Json(view))
}

/// 鎖住這筆預約並確認它仍可變更(尚未開始,且狀態允許);
/// 取消可用於待確認或已確認,改期只能用於已確認。回傳 (id, service_id, staff_user_id)
async fn lock_changeable(
    tx: &mut Tx,
    raw: &str,
    allow_pending: bool,
) -> Result<(Uuid, Uuid, Uuid, BookingStatus), AppError> {
    let row: Option<(Uuid, Uuid, Uuid, BookingStatus, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, service_id, staff_user_id, status, starts_at FROM bookings
         WHERE manage_token_hash = $1 FOR UPDATE",
    )
    .bind(token::hash(raw))
    .fetch_optional(&mut **tx)
    .await?;
    let (id, service_id, staff_id, status, starts_at) = row.ok_or(AppError::NotFound)?;
    let allowed =
        status == BookingStatus::Confirmed || (allow_pending && status == BookingStatus::Pending);
    if !allowed {
        return Err(AppError::Conflict("此預約目前無法變更".into()));
    }
    if starts_at <= Utc::now() {
        return Err(AppError::Conflict("預約已開始,無法變更".into()));
    }
    Ok((id, service_id, staff_id, status))
}
