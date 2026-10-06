//! 預約核心:載入排程、驗證時段、建立 / 改期。
//!
//! 計算可預約時段只是「建議」;真正的把關是 `bookings` 上的排除約束。
//! 這裡的驗證擋掉明顯無效的請求(過去、不在營業時間、員工不提供該服務…),
//! 資料庫負責擋掉併發下的重複預約。

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sqlx::{Acquire, FromRow};
use uuid::Uuid;

use crate::{
    availability::{SLOT_STEP_MINUTES, Slot, StaffSchedule, Timing, compute_slots},
    db::{PG_EXCLUSION_VIOLATION, Tx, pg_code},
    error::AppError,
    token,
    validation::normalize_email,
};

/// 最遠可預約幾天後
pub const MAX_ADVANCE_DAYS: i64 = 90;
/// 單一顧客同時最多有幾筆未來的有效(待確認或已確認)預約,防止被一個 Email 灌爆
pub const MAX_UPCOMING_PER_CUSTOMER: i64 = 5;
/// 待確認的預約超過幾小時未確認就失效(確認連結的有效期)
pub const PENDING_TTL_HOURS: i32 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "booking_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum BookingStatus {
    /// 顧客自助預約後、尚未按確認連結:不占時段
    Pending,
    Confirmed,
    Cancelled,
    Completed,
    NoShow,
}

#[derive(Debug, Clone, FromRow)]
pub struct ServiceInfo {
    pub id: Uuid,
    pub name: String,
    pub duration_minutes: i32,
    /// 服務結束後,員工多久不能接下一筆(顧客看不到)
    pub buffer_minutes: i32,
}

impl ServiceInfo {
    pub fn duration(&self) -> Duration {
        Duration::minutes(i64::from(self.duration_minutes))
    }

    pub fn buffer(&self) -> Duration {
        Duration::minutes(i64::from(self.buffer_minutes))
    }

    /// 預約結束後員工仍被占住到什麼時候(寫進 `bookings.blocked_until`,是預約當下的快照)
    pub fn blocked_until(&self, start: DateTime<Utc>) -> DateTime<Utc> {
        start + self.duration() + self.buffer()
    }
}

/// 啟用中的服務;不存在或已停用都回 400(對公開端點而言兩者沒有差別)
pub async fn active_service(tx: &mut Tx, service_id: Uuid) -> Result<ServiceInfo, AppError> {
    sqlx::query_as::<_, ServiceInfo>(
        "SELECT id, name, duration_minutes, buffer_minutes FROM services WHERE id = $1 AND active",
    )
    .bind(service_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::BadRequest("服務不存在或已停用".into()))
}

pub async fn tenant_timezone(tx: &mut Tx, tenant_id: Uuid) -> Result<Tz, AppError> {
    let name: String = sqlx::query_scalar("SELECT timezone FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .fetch_one(&mut **tx)
        .await?;
    name.parse::<Tz>()
        .map_err(|_| AppError::Internal(anyhow::anyhow!("無法辨識的時區: {name}")))
}

/// 載入「提供這項服務的員工」在 [range_start, range_end) 附近的營業時間與忙碌區間
async fn load_schedules(
    tx: &mut Tx,
    service_id: Uuid,
    staff_filter: Option<Uuid>,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    exclude_booking: Option<Uuid>,
) -> Result<Vec<StaffSchedule>, AppError> {
    let staff: Vec<Uuid> = sqlx::query_scalar(
        "SELECT ss.user_id FROM staff_services ss
         JOIN memberships m ON m.user_id = ss.user_id AND m.active
         WHERE ss.service_id = $1 AND ($2::uuid IS NULL OR ss.user_id = $2)
         ORDER BY ss.user_id",
    )
    .bind(service_id)
    .bind(staff_filter)
    .fetch_all(&mut **tx)
    .await?;
    if staff.is_empty() {
        return Ok(Vec::new());
    }

    let hours: Vec<(Uuid, i16, chrono::NaiveTime, chrono::NaiveTime)> = sqlx::query_as(
        "SELECT user_id, weekday, start_time, end_time FROM working_hours WHERE user_id = ANY($1)",
    )
    .bind(&staff)
    .fetch_all(&mut **tx)
    .await?;

    let time_off: Vec<(Uuid, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT user_id, starts_at, ends_at FROM time_off
           WHERE user_id = ANY($1) AND ends_at > $2 AND starts_at < $3",
    )
    .bind(&staff)
    .bind(range_start)
    .bind(range_end)
    .fetch_all(&mut **tx)
    .await?;
    // 既有預約占住到 blocked_until(結束 + 當時的整理時間),不是 ends_at
    let booked: Vec<(Uuid, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT staff_user_id, starts_at, blocked_until FROM bookings
           WHERE staff_user_id = ANY($1)
             AND status IN ('confirmed', 'completed', 'no_show')
             AND blocked_until > $2 AND starts_at < $3
             AND ($4::uuid IS NULL OR id <> $4)",
    )
    .bind(&staff)
    .bind(range_start)
    .bind(range_end)
    .bind(exclude_booking)
    .fetch_all(&mut **tx)
    .await?;

    Ok(staff
        .into_iter()
        .map(|user_id| StaffSchedule {
            user_id,
            hours: hours
                .iter()
                .filter(|(u, ..)| *u == user_id)
                .map(|&(_, wd, s, e)| (wd as u32, s, e))
                .collect(),
            time_off: time_off
                .iter()
                .filter(|(u, ..)| *u == user_id)
                .map(|&(_, s, e)| (s, e))
                .collect(),
            booked: booked
                .iter()
                .filter(|(u, ..)| *u == user_id)
                .map(|&(_, s, e)| (s, e))
                .collect(),
        })
        .collect())
}

/// 查詢 [from, to](店家本地日期)的可預約時段
pub async fn availability(
    tx: &mut Tx,
    tz: Tz,
    service: &ServiceInfo,
    from: NaiveDate,
    to: NaiveDate,
    staff_filter: Option<Uuid>,
    exclude_booking: Option<Uuid>,
) -> Result<Vec<Slot>, AppError> {
    // 多抓前後各一天,避免時區換算造成邊界遺漏
    let pad = Duration::days(1);
    let range_start = from.and_hms_opt(0, 0, 0).unwrap().and_utc() - pad - pad;
    let range_end = to.and_hms_opt(0, 0, 0).unwrap().and_utc() + pad + pad + pad;
    let schedules = load_schedules(
        tx,
        service.id,
        staff_filter,
        range_start,
        range_end,
        exclude_booking,
    )
    .await?;
    Ok(compute_slots(
        tz,
        from,
        to,
        Timing {
            duration: service.duration(),
            buffer: service.buffer(),
            step: Duration::minutes(SLOT_STEP_MINUTES),
        },
        Utc::now(),
        &schedules,
    ))
}

fn check_start_in_window(start: DateTime<Utc>) -> Result<(), AppError> {
    let now = Utc::now();
    if start <= now {
        return Err(AppError::BadRequest("預約時間必須在未來".into()));
    }
    if start > now + Duration::days(MAX_ADVANCE_DAYS) {
        return Err(AppError::BadRequest(format!(
            "最多只能預約 {MAX_ADVANCE_DAYS} 天內"
        )));
    }
    Ok(())
}

/// 確認待確認預約的那一刻,重新檢查這位員工在該時段仍然有空
/// (申請之後,員工可能新增了休假或改了營業時間)。排除自己。
pub async fn still_bookable(
    tx: &mut Tx,
    tz: Tz,
    booking_id: Uuid,
    service_id: Uuid,
    staff_id: Uuid,
    start: DateTime<Utc>,
) -> Result<bool, AppError> {
    // 服務可能已被停用:這時不該再成立新預約
    let service = match active_service(tx, service_id).await {
        Ok(s) => s,
        Err(AppError::BadRequest(_)) => return Ok(false),
        Err(e) => return Err(e),
    };
    let free = staff_free_at(tx, tz, &service, start, Some(staff_id), Some(booking_id)).await?;
    Ok(!free.is_empty())
}

/// 哪些員工在 `start` 這個時間點真的有空(依序,可直接拿來依序嘗試寫入)
async fn staff_free_at(
    tx: &mut Tx,
    tz: Tz,
    service: &ServiceInfo,
    start: DateTime<Utc>,
    staff_filter: Option<Uuid>,
    exclude_booking: Option<Uuid>,
) -> Result<Vec<Uuid>, AppError> {
    let date = start.with_timezone(&tz).date_naive();
    let slots = availability(tx, tz, service, date, date, staff_filter, exclude_booking).await?;
    Ok(slots
        .into_iter()
        .filter(|s| s.start == start)
        .map(|s| s.staff_id)
        .collect())
}

#[derive(Debug, Deserialize)]
pub struct CustomerInput {
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NewBooking {
    pub service_id: Uuid,
    pub staff_id: Option<Uuid>,
    pub start: DateTime<Utc>,
    pub customer: CustomerInput,
}

#[derive(Debug)]
pub struct Created {
    pub id: Uuid,
    pub customer_email: String,
    pub staff_id: Uuid,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    /// 原始管理連結 token,只此一次
    pub token: String,
}

/// `initial` 只能是 Pending(顧客自助,需 Email 確認)或 Confirmed(員工代客,員工即信任來源)。
/// Pending 不在排除約束的條件內,所以不占時段;時段會在確認那一刻才真正被占住。
pub async fn create_booking(
    tx: &mut Tx,
    tenant_id: Uuid,
    tz: Tz,
    input: NewBooking,
    initial: BookingStatus,
) -> Result<Created, AppError> {
    debug_assert!(matches!(
        initial,
        BookingStatus::Pending | BookingStatus::Confirmed
    ));
    check_start_in_window(input.start)?;

    let name = input.customer.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(AppError::BadRequest("姓名長度需為 1–100 字".into()));
    }
    let email = normalize_email(&input.customer.email)?;
    let phone = input
        .customer
        .phone
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    if phone.as_ref().is_some_and(|p| p.chars().count() > 30) {
        return Err(AppError::BadRequest("電話長度過長".into()));
    }

    // 員工代客預約直接占位,要序列化檢查;顧客自助(pending)不占位,只先擋掉明顯已額滿的申請
    crate::plan::ensure_booking_slot(
        tx,
        tenant_id,
        tz,
        input.start,
        initial == BookingStatus::Confirmed,
    )
    .await?;

    let service = active_service(tx, input.service_id).await?;
    let candidates = staff_free_at(tx, tz, &service, input.start, input.staff_id, None).await?;
    if candidates.is_empty() {
        return Err(AppError::Conflict("所選時段無法預約".into()));
    }

    // 既有顧客不覆寫姓名 / 電話:公開端點任何人都能填別人的 Email,不能讓它改掉別人的資料
    let customer_id: Uuid = sqlx::query_scalar(
        "INSERT INTO customers (tenant_id, name, email, phone) VALUES ($1, $2, $3::citext, $4)
         ON CONFLICT (tenant_id, email) DO UPDATE SET email = EXCLUDED.email
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(&name)
    .bind(&email)
    .bind(&phone)
    .fetch_one(&mut **tx)
    .await?;

    let upcoming: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM bookings
         WHERE customer_id = $1 AND status IN ('pending', 'confirmed') AND starts_at > now()",
    )
    .bind(customer_id)
    .fetch_one(&mut **tx)
    .await?;
    if upcoming >= MAX_UPCOMING_PER_CUSTOMER {
        return Err(AppError::Conflict(format!(
            "同一位顧客最多同時有 {MAX_UPCOMING_PER_CUSTOMER} 筆未來預約"
        )));
    }

    let ends_at = input.start + service.duration();
    for staff_id in candidates {
        let raw_token = token::generate();
        // savepoint:排除約束違規會讓整個交易進入錯誤狀態,要能退回去嘗試下一位員工
        let mut sp = tx.begin().await?;
        let result = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO bookings
               (tenant_id, staff_user_id, service_id, customer_id, starts_at, ends_at,
                manage_token_hash, status, confirmed_at, blocked_until)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
        )
        .bind(tenant_id)
        .bind(staff_id)
        .bind(service.id)
        .bind(customer_id)
        .bind(input.start)
        .bind(ends_at)
        .bind(token::hash(&raw_token))
        .bind(initial)
        .bind((initial == BookingStatus::Confirmed).then(Utc::now))
        .bind(service.blocked_until(input.start))
        .fetch_one(&mut *sp)
        .await;
        match result {
            Ok(id) => {
                sp.commit().await?;
                return Ok(Created {
                    id,
                    customer_email: email,
                    staff_id,
                    starts_at: input.start,
                    ends_at,
                    token: raw_token,
                });
            }
            Err(e) if pg_code(&e).as_deref() == Some(PG_EXCLUSION_VIOLATION) => {
                sp.rollback().await?;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(AppError::Conflict(
        "該時段剛被預約走了,請選擇其他時間".into(),
    ))
}

/// 改期:同一位員工、同一個服務,換到新的開始時間
pub async fn reschedule_booking(
    tx: &mut Tx,
    tenant_id: Uuid,
    tz: Tz,
    booking_id: Uuid,
    service: &ServiceInfo,
    staff_id: Uuid,
    new_start: DateTime<Utc>,
) -> Result<DateTime<Utc>, AppError> {
    // 回傳改期前的開始時間(寫進稽核)
    check_start_in_window(new_start)?;

    // 改到不同的月份就等於在那個月多占一個名額,不能用改期繞過每月上限
    let old_start: DateTime<Utc> =
        sqlx::query_scalar("SELECT starts_at FROM bookings WHERE id = $1")
            .bind(booking_id)
            .fetch_one(&mut **tx)
            .await?;
    if crate::plan::month_bounds(tz, old_start) != crate::plan::month_bounds(tz, new_start) {
        crate::plan::ensure_booking_slot(tx, tenant_id, tz, new_start, true).await?;
    }

    // 排除自己:否則同一時段內小幅移動(10:00 → 10:15)會和自己衝突
    let free = staff_free_at(tx, tz, service, new_start, Some(staff_id), Some(booking_id)).await?;
    if free.is_empty() {
        return Err(AppError::Conflict("所選時段無法預約".into()));
    }
    let new_end = new_start + service.duration();
    // reminder_queued_at 清掉、reminder_seq 加一:提醒是針對「那個時間」寄的,改期之後要針對新時間重新排
    let result = sqlx::query(
        "UPDATE bookings SET starts_at = $2, ends_at = $3, blocked_until = $4,
                reminder_queued_at = NULL, reminder_seq = reminder_seq + 1 WHERE id = $1",
    )
    .bind(booking_id)
    .bind(new_start)
    .bind(new_end)
    .bind(service.blocked_until(new_start))
    .execute(&mut **tx)
    .await;
    match result {
        Ok(_) => Ok(old_start),
        Err(e) if pg_code(&e).as_deref() == Some(PG_EXCLUSION_VIOLATION) => Err(
            AppError::Conflict("該時段剛被預約走了,請選擇其他時間".into()),
        ),
        Err(e) => Err(e.into()),
    }
}

/// 寄信需要的預約資訊
pub struct MailCtx {
    /// 顧客的 Email
    pub to: String,
    pub shop: String,
    /// 店家網址代稱(通知信裡放「重新預約」的連結)
    pub slug: String,
    pub service: String,
    pub staff: String,
    /// 負責這筆預約的員工的 Email(通知員工用)
    pub staff_email: String,
    pub customer_name: String,
    pub when: String,
}

impl MailCtx {
    pub fn view(&self) -> crate::mail::BookingMail<'_> {
        crate::mail::BookingMail {
            to: &self.to,
            shop: &self.shop,
            service: &self.service,
            staff: &self.staff,
            when: &self.when,
        }
    }
}

pub async fn mail_ctx(tx: &mut Tx, booking_id: Uuid) -> Result<MailCtx, AppError> {
    let (to, shop, slug, timezone, service, staff, staff_email, customer_name, starts_at): (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        DateTime<Utc>,
    ) = sqlx::query_as(
        "SELECT c.email::text, t.name, t.slug, t.timezone, s.name, u.name, u.email::text, c.name,
                b.starts_at
         FROM bookings b
         JOIN customers c ON c.id = b.customer_id
         JOIN tenants t ON t.id = b.tenant_id
         JOIN services s ON s.id = b.service_id
         JOIN users u ON u.id = b.staff_user_id
         WHERE b.id = $1",
    )
    .bind(booking_id)
    .fetch_one(&mut **tx)
    .await?;
    let tz: Tz = timezone
        .parse()
        .map_err(|_| AppError::Internal(anyhow::anyhow!("無法辨識的時區: {timezone}")))?;
    Ok(MailCtx {
        to,
        shop,
        slug,
        service,
        staff,
        staff_email,
        customer_name,
        when: crate::mail::format_local(starts_at, tz),
    })
}
