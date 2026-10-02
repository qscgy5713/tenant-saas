//! 訂閱方案與限額。
//!
//! 「先數再寫」在併發下會超賣,所以檢查前先拿 `pg_advisory_xact_lock`(依店家與資源種類),
//! 鎖到交易結束:數量檢查和接下來的寫入在同一個臨界區內完成。

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, Utc};
use chrono_tz::Tz;
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{availability::local_to_utc, db::Tx, error::AppError};

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Plan {
    pub id: String,
    pub name: String,
    pub price_cents: i32,
    pub max_staff: Option<i32>,
    pub max_services: Option<i32>,
    pub max_bookings_per_month: Option<i32>,
}

/// 店家目前的方案(限額即時讀取,方案升降級立刻生效)
pub async fn current(tx: &mut Tx, tenant_id: Uuid) -> Result<Plan, AppError> {
    Ok(sqlx::query_as::<_, Plan>(
        "SELECT p.id, p.name, p.price_cents, p.max_staff, p.max_services, p.max_bookings_per_month
         FROM tenants t JOIN plans p ON p.id = t.plan_id WHERE t.id = $1",
    )
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await?)
}

/// 取得「店家 × 資源」的交易層級鎖;key 的格式要和 `accept_invitation()` 一致
pub async fn lock(tx: &mut Tx, tenant_id: Uuid, kind: &str) -> Result<(), AppError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("quota:{kind}:{tenant_id}"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn exceeded(plan: &Plan, what: &str, max: i32) -> AppError {
    AppError::LimitReached(format!(
        "已達「{}」方案的{what}上限({max}),請升級方案",
        plan.name
    ))
}

/// 成員 + 待接受的邀請(邀請會預留名額)是否已滿。呼叫端須在同一交易內接著寫入。
pub async fn ensure_staff_slot(tx: &mut Tx, tenant_id: Uuid) -> Result<(), AppError> {
    lock(tx, tenant_id, "staff").await?;
    let plan = current(tx, tenant_id).await?;
    let Some(max) = plan.max_staff else {
        return Ok(());
    };
    let used: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM memberships)
              + (SELECT count(*) FROM invitations WHERE accepted_at IS NULL AND expires_at > now())",
    )
    .fetch_one(&mut **tx)
    .await?;
    if used >= i64::from(max) {
        return Err(exceeded(&plan, "成員人數", max));
    }
    Ok(())
}

/// 啟用中的服務是否已滿(建立新服務,或重新啟用已停用的服務時檢查)
pub async fn ensure_service_slot(tx: &mut Tx, tenant_id: Uuid) -> Result<(), AppError> {
    lock(tx, tenant_id, "services").await?;
    let plan = current(tx, tenant_id).await?;
    let Some(max) = plan.max_services else {
        return Ok(());
    };
    let used: i64 = sqlx::query_scalar("SELECT count(*) FROM services WHERE active")
        .fetch_one(&mut **tx)
        .await?;
    if used >= i64::from(max) {
        return Err(exceeded(&plan, "服務項目數", max));
    }
    Ok(())
}

/// `at` 所在的店家當地月份的起訖(UTC),半開區間 [start, end)
pub fn month_bounds(tz: Tz, at: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let local = at.with_timezone(&tz).date_naive();
    let first = local.with_day(1).expect("每個月都有 1 號");
    let next = if first.month() == 12 {
        NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    }
    .expect("下個月 1 號一定存在");
    let midnight = |d: NaiveDate| {
        // 少數時區的午夜可能因夏令時間不存在,往後找第一個存在的整點
        (0..4)
            .find_map(|h| local_to_utc(tz, d, NaiveTime::from_hms_opt(h, 0, 0)?))
            .unwrap_or_else(|| d.and_hms_opt(0, 0, 0).expect("合法時間").and_utc())
    };
    (midnight(first), midnight(next))
}

async fn bookings_in_month(tx: &mut Tx, tz: Tz, at: DateTime<Utc>) -> Result<i64, AppError> {
    let (start, end) = month_bounds(tz, at);
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM bookings
         WHERE status IN ('confirmed', 'completed', 'no_show') AND starts_at >= $1 AND starts_at < $2",
    )
    .bind(start)
    .bind(end)
    .fetch_one(&mut **tx)
    .await?)
}

/// `start` 所在月份是否還有預約名額。`lock` 為 true 時序列化(會占位的操作都要);
/// 只是提早擋掉「待確認」申請時不需要鎖。
pub async fn ensure_booking_slot(
    tx: &mut Tx,
    tenant_id: Uuid,
    tz: Tz,
    start: DateTime<Utc>,
    lock_it: bool,
) -> Result<(), AppError> {
    if lock_it {
        lock(tx, tenant_id, "bookings").await?;
    }
    let plan = current(tx, tenant_id).await?;
    let Some(max) = plan.max_bookings_per_month else {
        return Ok(());
    };
    if bookings_in_month(tx, tz, start).await? >= i64::from(max) {
        return Err(exceeded(&plan, "每月預約數", max));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct Usage {
    pub staff: i64,
    pub pending_invitations: i64,
    pub services: i64,
    pub bookings_this_month: i64,
}

pub async fn usage(tx: &mut Tx, tz: Tz) -> Result<Usage, AppError> {
    let (staff, pending_invitations, services): (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM memberships),
                (SELECT count(*) FROM invitations WHERE accepted_at IS NULL AND expires_at > now()),
                (SELECT count(*) FROM services WHERE active)",
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(Usage {
        staff,
        pending_invitations,
        services,
        bookings_this_month: bookings_in_month(tx, tz, Utc::now()).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn month_is_decided_by_the_shops_local_calendar_not_utc() {
        // 台北 2026-11-01 00:30 = UTC 2026-10-31 16:30:屬於 11 月,不是 10 月
        let (start, end) = month_bounds(chrono_tz::Asia::Taipei, utc("2026-10-31T16:30:00Z"));
        assert_eq!(start, utc("2026-10-31T16:00:00Z"));
        assert_eq!(end, utc("2026-11-30T16:00:00Z"));
        // 台北 2026-10-31 23:59 仍屬 10 月
        let (start, _) = month_bounds(chrono_tz::Asia::Taipei, utc("2026-10-31T15:59:00Z"));
        assert_eq!(start, utc("2026-09-30T16:00:00Z"));
    }

    #[test]
    fn december_rolls_into_next_year_and_dst_changes_the_offset() {
        let (_, end) = month_bounds(chrono_tz::UTC, utc("2026-12-15T00:00:00Z"));
        assert_eq!(end, utc("2027-01-01T00:00:00Z"));
        // 紐約 3 月:月初 EST(UTC-5),下月初 EDT(UTC-4)
        let (start, end) = month_bounds(chrono_tz::America::New_York, utc("2026-03-15T12:00:00Z"));
        assert_eq!(start, utc("2026-03-01T05:00:00Z"));
        assert_eq!(end, utc("2026-04-01T04:00:00Z"));
    }
}
