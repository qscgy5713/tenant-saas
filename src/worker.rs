//! 背景 worker:過期未確認的預約、排入提醒信、寄出 outbox 的信。
//!
//! 所有 SQL 都以 `tenant_worker` 角色執行(非超級使用者、無 BYPASSRLS,只有明確授權的資料表)。
//! 租戶上下文來自每一列資料本身的 `tenant_id`,不依賴任何請求狀態。

use std::time::Duration;

use chrono::Utc;
use chrono_tz::Tz;
use serde_json::json;
use sqlx::{FromRow, PgPool};
use tokio::sync::watch;
use uuid::Uuid;

use crate::{
    audit::{self, Actor},
    booking::PENDING_TTL_HOURS,
    db::begin_worker,
    error::AppError,
    mail::{BookingMail, Email, Mailer, booking_link, format_local, reminder},
    outbox, token,
};

pub const MAX_ATTEMPTS: i32 = 5;
const SEND_BATCH: usize = 50;
const SEND_TIMEOUT: Duration = Duration::from_secs(30);
const RETENTION_DAYS: i32 = 30;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TickStats {
    pub expired: u64,
    pub reminders: u64,
    pub sent: u64,
    pub retried: u64,
    pub failed: u64,
    pub cleaned: u64,
}

/// 各階段獨立:提醒排程或過期整理出錯,不能連帶讓寄信停擺。階段失敗只記錄,不中斷其他階段。
pub async fn tick(
    pool: &PgPool,
    mailer: &Mailer,
    public_base_url: &str,
) -> Result<TickStats, AppError> {
    fn or_log<T: Default>(stage: &str, result: Result<T, AppError>) -> T {
        result.unwrap_or_else(|err| {
            tracing::error!(stage, error = %err, "worker 階段失敗");
            T::default()
        })
    }

    let mut stats = TickStats {
        expired: or_log("expire_pending", expire_pending(pool).await),
        reminders: or_log(
            "enqueue_reminders",
            enqueue_reminders(pool, public_base_url).await,
        ),
        ..Default::default()
    };
    // 寄信失敗代表連資料庫都有問題,這個才往外回報
    send_due(pool, mailer, &mut stats).await?;
    stats.cleaned = or_log("cleanup", cleanup(pool).await);
    Ok(stats)
}

/// 超過有效期仍未確認的預約:取消(本來就不占時段,只是整理狀態),並寫入稽核
async fn expire_pending(pool: &PgPool) -> Result<u64, AppError> {
    let mut tx = begin_worker(pool).await?;
    let expired: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "UPDATE bookings SET status = 'cancelled'
         WHERE status = 'pending' AND created_at < now() - make_interval(hours => $1)
         RETURNING id, tenant_id",
    )
    .bind(PENDING_TTL_HOURS)
    .fetch_all(&mut *tx)
    .await?;
    for (booking_id, tenant_id) in &expired {
        audit::record(
            &mut tx,
            *tenant_id,
            Actor::System,
            "booking.cancelled",
            "booking",
            Some(*booking_id),
            json!({ "to": "cancelled", "reason": "confirmation_expired" }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(expired.len() as u64)
}

#[derive(FromRow)]
struct ReminderRow {
    id: Uuid,
    tenant_id: Uuid,
    starts_at: chrono::DateTime<Utc>,
    reminder_seq: i32,
    customer_email: String,
    service_name: String,
    staff_name: String,
    shop_name: String,
    timezone: String,
}

/// 24 小時內開始、且確認時距離開始超過 24 小時的預約,各排一封提醒信。
/// (確認時就已經不到 24 小時的,剛收過確認信,不再重複提醒。)
async fn enqueue_reminders(pool: &PgPool, public_base_url: &str) -> Result<u64, AppError> {
    let mut tx = begin_worker(pool).await?;
    let rows = sqlx::query_as::<_, ReminderRow>(
        "SELECT b.id, b.tenant_id, b.starts_at, b.reminder_seq,
                c.email::text AS customer_email, s.name AS service_name,
                u.name AS staff_name, t.name AS shop_name, t.timezone
         FROM bookings b
         JOIN customers c ON c.id = b.customer_id
         JOIN services s ON s.id = b.service_id
         JOIN users u ON u.id = b.staff_user_id
         JOIN tenants t ON t.id = b.tenant_id
         WHERE b.status = 'confirmed'
           AND b.reminder_queued_at IS NULL
           AND b.starts_at > now() AND b.starts_at <= now() + interval '24 hours'
           AND b.starts_at - b.confirmed_at > interval '24 hours'
           AND t.status = 'active'
         ORDER BY b.starts_at
         LIMIT 100
         FOR UPDATE OF b SKIP LOCKED",
    )
    .fetch_all(&mut *tx)
    .await?;

    let mut queued = 0;
    for row in rows {
        let Ok(tz) = row.timezone.parse::<Tz>() else {
            tracing::error!(booking_id = %row.id, "無法辨識店家時區,略過提醒");
            continue;
        };
        let when = format_local(row.starts_at, tz);
        // 提醒信專用的管理 token:原始值只在這封信裡,資料庫只存雜湊
        let raw = token::generate();
        let email = reminder(
            &BookingMail {
                to: &row.customer_email,
                shop: &row.shop_name,
                service: &row.service_name,
                staff: &row.staff_name,
                when: &when,
            },
            &booking_link(public_base_url, &raw),
        );
        outbox::enqueue(
            &mut tx,
            row.tenant_id,
            &email,
            // 去重鍵含排程輪次:每次改期遞增,所以改期後會再提醒一次。
            // 不用開始時間:A → B → 再改回 A 時,鍵會與第一次相同而被去重吞掉
            Some(&format!("reminder:{}:{}", row.id, row.reminder_seq)),
        )
        .await?;
        sqlx::query(
            "UPDATE bookings SET reminder_queued_at = now(), reminder_token_hash = $2 WHERE id = $1",
        )
        .bind(row.id)
        .bind(token::hash(&raw))
            .execute(&mut *tx)
            .await?;
        queued += 1;
    }
    tx.commit().await?;
    Ok(queued)
}

#[derive(FromRow)]
struct OutboxRow {
    id: Uuid,
    to_email: String,
    subject: String,
    body: String,
    attempts: i32,
}

/// 逐封處理:每封信一個交易,只在寄送期間鎖住那一列(`SKIP LOCKED`,多個 worker 不會重複寄)
async fn send_due(pool: &PgPool, mailer: &Mailer, stats: &mut TickStats) -> Result<(), AppError> {
    for _ in 0..SEND_BATCH {
        let mut tx = begin_worker(pool).await?;
        let row = sqlx::query_as::<_, OutboxRow>(
            "SELECT id, to_email, subject, body, attempts FROM email_outbox
             WHERE status = 'pending' AND run_at <= now()
             ORDER BY run_at LIMIT 1
             FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.rollback().await?;
            return Ok(());
        };

        let email = Email {
            to: row.to_email.clone(),
            subject: row.subject.clone(),
            body: row.body.clone(),
        };
        let result = match tokio::time::timeout(SEND_TIMEOUT, mailer.send(&email)).await {
            Ok(r) => r,
            Err(_) => Err(anyhow::anyhow!("寄送逾時")),
        };

        let attempts = row.attempts + 1;
        match result {
            Ok(()) => {
                // 寄出後清空內容:信裡有一次性連結,不長期留在資料庫
                sqlx::query(
                    "UPDATE email_outbox SET status = 'sent', sent_at = now(), body = '',
                            attempts = $2, last_error = NULL WHERE id = $1",
                )
                .bind(row.id)
                .bind(attempts)
                .execute(&mut *tx)
                .await?;
                stats.sent += 1;
            }
            Err(err) => {
                let message = format!("{err:#}");
                tracing::warn!(outbox_id = %row.id, attempts, error = %message, "寄信失敗");
                if attempts >= MAX_ATTEMPTS {
                    sqlx::query(
                        "UPDATE email_outbox SET status = 'failed', body = '',
                                attempts = $2, last_error = $3 WHERE id = $1",
                    )
                    .bind(row.id)
                    .bind(attempts)
                    .bind(&message)
                    .execute(&mut *tx)
                    .await?;
                    stats.failed += 1;
                } else {
                    // 指數退避:30 秒、1 分、2 分、4 分。用資料庫的時鐘計算,多台機器時鐘不同步也不影響
                    let delay_secs = 30.0 * f64::from(1_u32 << (attempts - 1));
                    sqlx::query(
                        "UPDATE email_outbox
                         SET attempts = $2, last_error = $3, run_at = now() + make_interval(secs => $4)
                         WHERE id = $1",
                    )
                    .bind(row.id)
                    .bind(attempts)
                    .bind(&message)
                    .bind(delay_secs)
                    .execute(&mut *tx)
                    .await?;
                    stats.retried += 1;
                }
            }
        }
        tx.commit().await?;
    }
    Ok(())
}

async fn cleanup(pool: &PgPool) -> Result<u64, AppError> {
    let mut tx = begin_worker(pool).await?;
    let n = sqlx::query(
        "DELETE FROM email_outbox
         WHERE status <> 'pending' AND created_at < now() - make_interval(days => $1)",
    )
    .bind(RETENTION_DAYS)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(n)
}

/// 保留政策的清理一次最多處理幾筆(一次做不完,下一輪接著做)
const RETENTION_BATCH: i32 = 200;
/// 保留政策的清理多久跑一次(不需要每個 tick 都跑)
const RETENTION_EVERY: Duration = Duration::from_secs(3600);
/// 刪除店家的寬限期(天)
pub const TENANT_DELETION_GRACE_DAYS: i32 = 30;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RetentionStats {
    pub customers_anonymized: u64,
    pub audit_rows_purged: u64,
    pub tenants_deleted: u64,
}

/// 資料保留政策:匿名化過期的顧客、清除過期的稽核日誌、刪除寬限期已滿的店家。
/// 各階段獨立,一個失敗不影響其他。每個階段一次最多處理 `RETENTION_BATCH` 筆,做不完下一輪再做。
pub async fn retention_sweep(pool: &PgPool, audit_days: Option<u32>) -> RetentionStats {
    fn or_log<T: Default>(stage: &str, result: Result<T, AppError>) -> T {
        result.unwrap_or_else(|err| {
            tracing::error!(stage, error = %err, "保留政策階段失敗");
            T::default()
        })
    }
    RetentionStats {
        customers_anonymized: or_log(
            "anonymize_customers",
            anonymize_expired_customers(pool).await,
        ),
        audit_rows_purged: match audit_days {
            Some(days) => or_log("purge_audit", purge_audit(pool, days).await),
            None => 0,
        },
        tenants_deleted: or_log("delete_tenants", delete_due_tenants(pool).await),
    }
}

/// 找出超過各店保留天數的顧客,逐一交給資料庫函式(函式會自己再確認一次,不信任這份名單)
async fn anonymize_expired_customers(pool: &PgPool) -> Result<u64, AppError> {
    let mut tx = begin_worker(pool).await?;
    let candidates: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT c.id, c.tenant_id
         FROM customers c JOIN tenants t ON t.id = c.tenant_id
         WHERE c.email::text NOT LIKE '%@anonymized.invalid'
           AND NOT EXISTS (SELECT 1 FROM bookings b WHERE b.customer_id = c.id
                           AND (b.status = 'pending' OR (b.status = 'confirmed' AND b.starts_at > now())))
           AND COALESCE((SELECT max(b.starts_at) FROM bookings b WHERE b.customer_id = c.id), c.created_at)
               < now() - make_interval(days => t.customer_retention_days)
         ORDER BY c.created_at LIMIT $1",
    )
    .bind(i64::from(RETENTION_BATCH))
    .fetch_all(&mut *tx)
    .await?;
    let mut done = 0;
    for (customer, tenant) in candidates {
        // 每位顧客各自一個 savepoint:某一位出錯(例如資料有問題)只跳過他,不讓整批回滾、
        // 也不讓同一位排在最前面的壞資料每小時都擋住後面的人
        let mut sp = sqlx::Acquire::begin(&mut tx).await?;
        let did: Result<bool, sqlx::Error> =
            sqlx::query_scalar("SELECT retention_anonymize_customer($1, $2)")
                .bind(customer)
                .bind(tenant)
                .fetch_one(&mut *sp)
                .await;
        match did {
            Ok(did) => {
                sp.commit().await?;
                if did {
                    done += 1;
                    // 只記 id:從備份還原後,靠日誌知道哪些顧客需要重新匿名化(其實保留政策會自動再做一次,
                    // 但店主手動刪除的不會)
                    tracing::info!(customer_id = %customer, tenant_id = %tenant, "依保留政策匿名化顧客");
                }
            }
            Err(err) => {
                sp.rollback().await?;
                metrics::counter!("customers_anonymize_failed_total").increment(1);
                tracing::error!(customer_id = %customer, error = %err, "匿名化顧客失敗,略過");
            }
        }
    }
    tx.commit().await?;
    if done > 0 {
        metrics::counter!("customers_anonymized_total").increment(done);
        tracing::info!(count = done, "依保留政策匿名化顧客");
    }
    Ok(done)
}

async fn purge_audit(pool: &PgPool, days: u32) -> Result<u64, AppError> {
    let mut tx = begin_worker(pool).await?;
    let n: i32 = sqlx::query_scalar("SELECT retention_purge_audit($1, $2)")
        .bind(i32::try_from(days).unwrap_or(i32::MAX))
        .bind(RETENTION_BATCH * 25)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    if n > 0 {
        metrics::counter!("audit_rows_purged_total").increment(n as u64);
        tracing::info!(rows = n, "依保留政策清除過期的稽核日誌");
    }
    Ok(n as u64)
}

async fn delete_due_tenants(pool: &PgPool) -> Result<u64, AppError> {
    let mut tx = begin_worker(pool).await?;
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT retention_delete_tenants($1)")
        .bind(10)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    for id in &ids {
        // 只記 id:店家的名稱與代稱也是申請刪除的資料,不留在日誌
        tracing::warn!(tenant_id = %id, "寬限期已滿,店家與其所有資料已刪除");
    }
    if !ids.is_empty() {
        metrics::counter!("tenants_deleted_total").increment(ids.len() as u64);
    }
    Ok(ids.len() as u64)
}

/// 常駐迴圈:每隔 `poll` 跑一次 `tick`,收到關閉訊號就結束
pub async fn run(
    pool: PgPool,
    mailer: Mailer,
    public_base_url: String,
    poll: Duration,
    audit_retention_days: Option<u32>,
    mut shutdown: watch::Receiver<bool>,
) {
    tracing::info!(?poll, "背景 worker 啟動");
    let mut last_sweep: Option<std::time::Instant> = None;
    while !*shutdown.borrow() {
        if last_sweep.is_none_or(|t| t.elapsed() >= RETENTION_EVERY) {
            last_sweep = Some(std::time::Instant::now());
            let stats = retention_sweep(&pool, audit_retention_days).await;
            if stats != RetentionStats::default() {
                tracing::info!(?stats, "保留政策清理");
            }
        }
        match tick(&pool, &mailer, &public_base_url).await {
            Ok(stats) if stats != TickStats::default() => {
                metrics::counter!("emails_sent_total").increment(stats.sent);
                metrics::counter!("emails_retried_total").increment(stats.retried);
                metrics::counter!("emails_failed_total").increment(stats.failed);
                tracing::info!(?stats, "worker tick");
            }
            Ok(_) => {}
            Err(err) => {
                metrics::counter!("worker_tick_errors_total").increment(1);
                tracing::error!(error = %err, "worker tick 失敗");
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(poll) => {}
            _ = shutdown.changed() => {}
        }
    }
    tracing::info!("背景 worker 已停止");
}
