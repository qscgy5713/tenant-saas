//! 啟動時檢查「應用程式的資料庫連線帳號」是否夠受限。
//!
//! 租戶隔離靠 RLS,而超級使用者、BYPASSRLS 角色、資料表擁有者都會繞過它。
//! 這個專案的做法是:連線帳號本身沒有任何租戶資料表的權限,每個請求都先 `SET LOCAL ROLE tenant_app`
//! 才碰資料。這樣即使哪天有人忘了經過 `begin_scoped` 直接對連線池查詢,
//! 也會得到 permission denied(失敗即關閉),而不是悄悄讀到別家店的資料。

use anyhow::Result;
use sqlx::PgPool;

/// 連線帳號不該直接讀取的表(必須經由 `SET LOCAL ROLE` 之後才能碰)
const TENANT_TABLES: &[&str] = &[
    "tenants",
    "memberships",
    "invitations",
    "services",
    "staff_services",
    "working_hours",
    "time_off",
    "customers",
    "bookings",
    "email_outbox",
    "audit_logs",
];

/// 回傳所有問題的說明;空陣列表示帳號夠受限
pub async fn problems(pool: &PgPool) -> Result<Vec<String>> {
    let (superuser, bypass_rls, owns_tables): (bool, bool, bool) = sqlx::query_as(
        "SELECT r.rolsuper, r.rolbypassrls,
                EXISTS (SELECT 1 FROM pg_tables WHERE schemaname = 'public' AND tableowner = current_user)
         FROM pg_roles r WHERE r.rolname = current_user",
    )
    .fetch_one(pool)
    .await?;

    let readable: Vec<String> = sqlx::query_scalar(
        "SELECT t FROM unnest($1::text[]) AS t
         WHERE CASE WHEN to_regclass('public.' || t) IS NULL THEN false
                    ELSE has_table_privilege(current_user, 'public.' || t, 'SELECT') END",
    )
    .bind(TENANT_TABLES)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::new();
    if superuser {
        out.push("連線帳號是超級使用者,會繞過所有 RLS 與權限檢查".to_string());
    }
    if bypass_rls {
        out.push("連線帳號有 BYPASSRLS".to_string());
    }
    if owns_tables {
        out.push("連線帳號是資料表擁有者(擁有者預設不受 RLS 約束)".to_string());
    }
    if !readable.is_empty() {
        out.push(format!(
            "連線帳號可以直接讀取租戶資料表:{}。應只能在 SET ROLE 之後存取",
            readable.join("、")
        ));
    }
    Ok(out)
}
