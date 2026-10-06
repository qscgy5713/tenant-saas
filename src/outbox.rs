use uuid::Uuid;

use crate::{db::Tx, error::AppError, mail::Email};

/// 把信排進 outbox。必須在和業務資料相同的交易內呼叫:交易成功才會寄,失敗就一起消失。
/// `dedupe_key` 相同的信只會排一次。
pub async fn enqueue(
    tx: &mut Tx,
    tenant_id: Uuid,
    email: &Email,
    dedupe_key: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO email_outbox (tenant_id, to_email, subject, body, dedupe_key)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (dedupe_key) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(&email.to)
    .bind(&email.subject)
    .bind(&email.body)
    .bind(dedupe_key)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 這個收件者這一小時內收到的信是否還沒超過上限(跨所有店家)。
/// 要在「寄給外部指定的收件者」之前呼叫(公開預約的驗證信、邀請信、店家代客預約的確認信):
/// 這些收件者不是系統能信任的人,不設限就能被拿來騷擾別人的信箱。
/// 檢查會對該收件者加鎖到交易結束,所以呼叫端必須在**同一個交易**內接著 `enqueue`。
pub async fn ensure_recipient_quota(tx: &mut Tx, to: &str, max: u32) -> Result<(), AppError> {
    let ok: bool = sqlx::query_scalar("SELECT mail_quota_ok($1, $2)")
        .bind(to)
        .bind(i32::try_from(max).unwrap_or(i32::MAX))
        .fetch_one(&mut **tx)
        .await?;
    if ok {
        Ok(())
    } else {
        Err(AppError::TooManyRequests)
    }
}
