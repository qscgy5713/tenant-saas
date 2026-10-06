//! 公開預約頁:顧客不需帳號。
//!
//! 租戶由網址代稱或預約連結 token 決定,解析後照一般流程設定租戶上下文,仍受 RLS 約束。
//! 這裡的 handler 只回傳公開資訊(服務、員工姓名、可預約時段),
//! 絕不回傳其他顧客的資料。

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
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
    ical, mail, outbox, plan, token,
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
        .route(
            "/public/shops/{slug}/bookings/{id}/resend",
            post(resend_verification),
        )
        .route("/public/bookings/{token}", get(get_booking))
        .route("/public/bookings/{token}/calendar.ics", get(booking_ics))
        .route(
            "/public/bookings/{token}/availability",
            get(booking_availability),
        )
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
         JOIN memberships m ON m.user_id = ss.user_id AND m.active
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

/// 日期範圍的共同檢查(結束不可早於開始,一次最多 14 天)。員工端與顧客端的改期查詢共用
pub(super) fn validated_range(
    from: NaiveDate,
    to: Option<NaiveDate>,
) -> Result<NaiveDate, AppError> {
    let to = to.unwrap_or(from);
    if to < from {
        return Err(AppError::BadRequest("結束日期不可早於開始日期".into()));
    }
    if (to - from).num_days() >= MAX_RANGE_DAYS {
        return Err(AppError::BadRequest(format!(
            "一次最多查詢 {MAX_RANGE_DAYS} 天"
        )));
    }
    Ok(to)
}

/// 改期用的查詢:不需要指定服務與人員(固定是這筆預約的),只要日期範圍
#[derive(Debug, Deserialize)]
pub(super) struct RangeQuery {
    pub from: NaiveDate,
    pub to: Option<NaiveDate>,
}

#[derive(Debug, Deserialize)]
struct AvailabilityQuery {
    service_id: Uuid,
    /// 店家本地日期 YYYY-MM-DD
    from: NaiveDate,
    to: Option<NaiveDate>,
    staff_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub(super) struct AvailabilityResponse {
    pub timezone: String,
    pub slots: Vec<Slot>,
}

async fn availability(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<AvailabilityQuery>,
) -> Result<Json<AvailabilityResponse>, AppError> {
    let to = validated_range(q.from, q.to)?;
    let (mut tx, _, tz) = open_shop(&state, &slug).await?;
    let service = booking::active_service(&mut tx, q.service_id).await?;
    let slots = booking::availability(&mut tx, tz, &service, q.from, to, q.staff_id, None).await?;
    tx.rollback().await?;
    Ok(Json(AvailabilityResponse {
        timezone: tz.name().to_string(),
        slots,
    }))
}

/// 這筆預約改期時可選的時段:同一位員工、同一個服務,而且**排除這筆預約自己**
/// (否則自己原本的時段會顯示成忙碌,連小幅調整 10:00 → 10:15 都選不到)。
async fn booking_availability(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    Query(q): Query<RangeQuery>,
) -> Result<Json<AvailabilityResponse>, AppError> {
    let to = validated_range(q.from, q.to)?;
    let (mut tx, _, tz) = open_by_token(&state, &raw).await?;
    let row: Option<(Uuid, Uuid, Uuid, BookingStatus)> = sqlx::query_as(
        "SELECT id, service_id, staff_user_id, status FROM bookings
         WHERE manage_token_hash = $1 OR reminder_token_hash = $1",
    )
    .bind(token::hash(&raw))
    .fetch_optional(&mut *tx)
    .await?;
    let (id, service_id, staff_id, status) = row.ok_or(AppError::NotFound)?;
    if status != BookingStatus::Confirmed {
        return Err(AppError::Conflict("只有已確認的預約可以改期".into()));
    }
    let service = booking::active_service(&mut tx, service_id).await?;
    let slots =
        booking::availability(&mut tx, tz, &service, q.from, to, Some(staff_id), Some(id)).await?;
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
    // 收件者是任何人都能填的 Email:限制它每小時能收幾封(跨店家),免得被拿來騷擾別人的信箱
    outbox::ensure_recipient_quota(&mut tx, &email.to, state.max_mails_per_recipient_per_hour)
        .await?;
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

/// 重寄確認信的上限與最短間隔
const MAX_RESENDS: i32 = 3;
const MIN_RESEND_GAP_SECS: i64 = 60;

#[derive(Debug, Deserialize)]
struct ResendBody {
    email: String,
}

#[derive(Debug, Serialize)]
struct Resent {
    message: &'static str,
}

/// 沒收到確認信時重寄。顧客沒收到信就沒有 token,所以用「預約編號 + Email」確認身分。
/// **不論有沒有真的寄出都回一樣的 202**(編號不對、Email 不符、已確認 / 已過期、超過次數或太頻繁),
/// 不洩漏任何預約是否存在。重寄會換新的 token,舊信裡的連結隨即失效。
async fn resend_verification(
    State(state): State<AppState>,
    Path((slug, id)): Path<(String, Uuid)>,
    Json(req): Json<ResendBody>,
) -> Result<(StatusCode, Json<Resent>), AppError> {
    let reply = || {
        (
            StatusCode::ACCEPTED,
            Json(Resent {
                message: "如果資料正確,確認信已重新寄出。請查看信箱(包含垃圾郵件匣)。",
            }),
        )
    };
    let email = req.email.trim().to_lowercase();
    let (mut tx, tenant_id, _) = open_shop(&state, &slug).await?;

    let row: Option<(BookingStatus, DateTime<Utc>, i32, DateTime<Utc>)> = sqlx::query_as(
        "SELECT b.status, b.created_at, b.verification_resends, b.verification_sent_at
         FROM bookings b JOIN customers c ON c.id = b.customer_id
         WHERE b.id = $1 AND lower(c.email::text) = $2
         FOR UPDATE OF b",
    )
    .bind(id)
    .bind(&email)
    .fetch_optional(&mut *tx)
    .await?;
    let now = Utc::now();
    let Some((status, created_at, resends, sent_at)) = row else {
        return Ok(reply());
    };
    let expired = created_at < now - Duration::hours(i64::from(PENDING_TTL_HOURS));
    let too_soon = sent_at > now - Duration::seconds(MIN_RESEND_GAP_SECS);
    if status != BookingStatus::Pending || expired || resends >= MAX_RESENDS || too_soon {
        return Ok(reply());
    }

    let raw = token::generate();
    sqlx::query(
        "UPDATE bookings SET manage_token_hash = $2, verification_resends = verification_resends + 1,
                verification_sent_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(token::hash(&raw))
    .execute(&mut *tx)
    .await?;

    let ctx = booking::mail_ctx(&mut tx, id).await?;
    let link = mail::booking_link(&state.public_base_url, &raw);
    let email = mail::verification(&ctx.view(), &link);
    // 超過上限就當作不寄(與其他「不寄」的情況一樣回相同的回應,不洩漏預約是否存在)
    if outbox::ensure_recipient_quota(&mut tx, &email.to, state.max_mails_per_recipient_per_hour)
        .await
        .is_err()
    {
        return Ok(reply());
    }
    outbox::enqueue(
        &mut tx,
        tenant_id,
        &email,
        Some(&format!("verify:{id}:{}", resends + 1)),
    )
    .await?;
    audit::record(
        &mut tx,
        tenant_id,
        Actor::Customer,
        "booking.verification_resent",
        "booking",
        Some(id),
        json!({ "count": resends + 1 }),
    )
    .await?;
    tx.commit().await?;
    Ok(reply())
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
         WHERE b.manage_token_hash = $1 OR b.reminder_token_hash = $1",
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
struct IcsRow {
    id: Uuid,
    status: BookingStatus,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    sequence: i32,
    shop: String,
    service: String,
    staff: String,
}

/// 下載 .ics,把已確認的預約加進自己的行事曆。UID 固定、SEQUENCE 隨改期遞增,
/// 所以改期後重新下載會「更新」行事曆裡的同一個事件,不會多出一筆。
async fn booking_ics(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let (mut tx, _, _) = open_by_token(&state, &raw).await?;
    let row = sqlx::query_as::<_, IcsRow>(
        "SELECT b.id, b.status, b.starts_at, b.ends_at, b.reminder_seq AS sequence,
                t.name AS shop, s.name AS service, u.name AS staff
         FROM bookings b
         JOIN tenants t ON t.id = b.tenant_id
         JOIN services s ON s.id = b.service_id
         JOIN users u ON u.id = b.staff_user_id
         WHERE b.manage_token_hash = $1 OR b.reminder_token_hash = $1",
    )
    .bind(token::hash(&raw))
    .fetch_optional(&mut *tx)
    .await?;
    tx.rollback().await?;
    let row = row.ok_or(AppError::NotFound)?;
    if row.status != BookingStatus::Confirmed {
        return Err(AppError::Conflict("只有已確認的預約可以加入行事曆".into()));
    }
    let body = ical::calendar(
        &ical::Event {
            uid: &format!("booking-{}@tenant-saas", row.id),
            sequence: row.sequence,
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            summary: &format!("{} - {}", row.service, row.shop),
            description: &format!(
                "店家:{}\n服務:{}\n人員:{}",
                row.shop, row.service, row.staff
            ),
        },
        Utc::now(),
    );
    Ok((
        [
            (header::CONTENT_TYPE, "text/calendar; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"booking.ics\"",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    ))
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
         FROM bookings WHERE manage_token_hash = $1 OR reminder_token_hash = $1 FOR UPDATE",
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
    // 已確認的預約被顧客取消 → 通知負責的員工(待確認的從沒進過員工的行事曆,不用通知)
    if from == BookingStatus::Confirmed {
        let m = booking::mail_ctx(&mut tx, id).await?;
        let email = mail::cancelled_for_staff(
            &m.view(),
            &m.staff_email,
            &m.customer_name,
            mail::CancelledBy::Customer,
        );
        outbox::enqueue(
            &mut tx,
            tenant_id,
            &email,
            Some(&format!("cancelled:{id}:staff")),
        )
        .await?;
    }
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
         WHERE manage_token_hash = $1 OR reminder_token_hash = $1 FOR UPDATE",
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
