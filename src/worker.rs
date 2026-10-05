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

/// 常駐迴圈:每隔 `poll` 跑一次 `tick`,收到關閉訊號就結束
pub async fn run(
    pool: PgPool,
    mailer: Mailer,
    public_base_url: String,
    poll: Duration,
    mut shutdown: watch::Receiver<bool>,
) {
    tracing::info!(?poll, "背景 worker 啟動");
    while !*shutdown.borrow() {
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
