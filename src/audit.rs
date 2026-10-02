//! 稽核日誌。
//!
//! 必須在和變更相同的交易內呼叫:交易提交,紀錄才存在;交易回滾,紀錄一起消失。
//! `detail` 只放識別碼、狀態轉換、欄位名稱等非敏感資訊——不放 Email、電話、token、休假原因,
//! 因為稽核紀錄只能新增、不能刪除,寫進去的個資就收不回來了。

use serde_json::{Value, json};
use uuid::Uuid;

use crate::{db::Tx, error::AppError};

#[derive(Debug, Clone, Copy)]
pub enum Actor {
    /// 登入的員工
    User(Uuid),
    /// 透過預約連結操作的顧客(沒有帳號)
    Customer,
    /// 背景 worker 等系統行為
    System,
}

impl Actor {
    fn parts(self) -> (&'static str, Option<Uuid>) {
        match self {
            Actor::User(id) => ("user", Some(id)),
            Actor::Customer => ("customer", None),
            Actor::System => ("system", None),
        }
    }
}

pub async fn record(
    tx: &mut Tx,
    tenant_id: Uuid,
    actor: Actor,
    action: &str,
    entity_type: &str,
    entity_id: Option<Uuid>,
    detail: Value,
) -> Result<(), AppError> {
    let (actor_type, actor_user_id) = actor.parts();
    sqlx::query(
        "INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(tenant_id)
    .bind(actor_type)
    .bind(actor_user_id)
    .bind(action)
    .bind(entity_type)
    .bind(entity_id)
    .bind(detail)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 沒有額外資訊時的簡寫
pub fn empty() -> Value {
    json!({})
}
