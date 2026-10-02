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
