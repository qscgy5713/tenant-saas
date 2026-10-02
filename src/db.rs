use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub type Tx = Transaction<'static, Postgres>;

/// 開啟一個「已套用租戶隔離」的交易。
///
/// 1. `SET LOCAL ROLE tenant_app`:切成受 RLS 約束的角色(資料表擁有者預設會繞過 RLS)
/// 2. 設定 `app.user_id` / `app.tenant_id`:`set_config(..., true)` 只在本交易有效,
///    連線歸還連線池後不會殘留到下一個請求
///
/// 所有碰租戶資料的查詢都必須經過這裡,不要直接對 `PgPool` 查詢租戶表。
pub async fn begin_scoped(
    pool: &PgPool,
    user_id: Option<Uuid>,
    tenant_id: Option<Uuid>,
) -> Result<Tx, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL ROLE tenant_app")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "SELECT set_config('app.user_id', $1, true), set_config('app.tenant_id', $2, true)",
    )
    .bind(user_id.map(|id| id.to_string()).unwrap_or_default())
    .bind(tenant_id.map(|id| id.to_string()).unwrap_or_default())
    .execute(&mut *tx)
    .await?;
    Ok(tx)
}

pub const PG_UNIQUE_VIOLATION: &str = "23505";
pub const PG_FOREIGN_KEY_VIOLATION: &str = "23503";
pub const PG_EXCLUSION_VIOLATION: &str = "23P01";

/// 取出資料庫錯誤的 SQLSTATE
pub fn pg_code(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.to_string())
}
