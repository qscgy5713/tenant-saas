//! 營運人員的維運指令:`tenant-saas admin <指令>`。
//!
//! 為什麼是子命令而不是網頁後台:營運人員只有少數幾位,命令列沒有新的網路攻擊面(不需要另一套登入與權限),
//! 而且映像裡本來就有這個執行檔(`docker compose run --rm app admin shops`)。
//! 用 `MIGRATION_DATABASE_URL`(擁有資料表的帳號)連線 —— runtime 帳號讀不到租戶資料表,這是設計上的。
//!
//! 只用「不是 FORCE ROW LEVEL SECURITY」的資料表(tenants、memberships、bookings、plans…):
//! customers / services 等 FORCE 的資料表連擁有者都要先設租戶上下文才讀得到,維運指令不需要它們。
//! 每個會改資料的指令都寫一筆稽核(actor = system,動作以 `operator.` 開頭),店主在稽核頁看得到。

use std::io::Write;

use anyhow::{Result, bail};
use serde_json::json;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

const USAGE: &str = "用法:tenant-saas admin <指令>
  plans                         列出方案與限額
  shops [數量]                  列出店家(最新的在前,預設 50):方案、狀態、成員數、近 30 天預約數
  show <代稱>                   一家店的詳細資料(店主、方案、用量、刪除 / 訂閱狀態)
  set-plan <代稱> <方案>        變更方案(會寫稽核)
  suspend <代稱>                暫停店家:後台與公開預約頁都進不去(不影響資料與訂閱)
  unsuspend <代稱>              恢復被暫停的店家
  export-audit <天數>           把超過這麼多天的稽核日誌以 JSON Lines 輸出到標準輸出(清除前先歸檔用;只讀)";

/// 執行一個指令,輸出寫到 `out`(正式執行是標準輸出;測試是記憶體)。
/// 歸檔可能很大,所以是邊讀邊寫,不會整份先載進記憶體。
pub async fn run<W: Write>(db: &PgPool, args: &[String], out: &mut W) -> Result<()> {
    if let (Some("export-audit"), Some(days), None) = (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
        args.get(2),
    ) {
        let days = days
            .parse::<i32>()
            .ok()
            .filter(|n| (1..=36_500).contains(n))
            .ok_or_else(|| anyhow::anyhow!("天數必須是 1 到 36500 的整數"))?;
        let n = export_audit(db, days, out).await?;
        // 筆數寫到標準錯誤:標準輸出只有資料,才能直接接 gzip
        eprintln!("已輸出 {n} 筆(超過 {days} 天的稽核日誌)");
        return Ok(());
    }
    let text = run_text(db, args).await?;
    out.write_all(text.as_bytes())?;
    Ok(())
}

async fn run_text(db: &PgPool, args: &[String]) -> Result<String> {
    let mut it = args.iter().map(String::as_str);
    match (it.next(), it.next(), it.next(), it.next()) {
        (Some("plans"), None, ..) => plans(db).await,
        (Some("shops"), limit, None, _) => {
            let limit = match limit {
                Some(v) => v.parse::<i64>().ok().filter(|n| (1..=1000).contains(n)),
                None => Some(50),
            };
            match limit {
                Some(n) => shops(db, n).await,
                None => bail!("數量必須是 1 到 1000 的整數"),
            }
        }
        (Some("show"), Some(slug), None, _) => show(db, slug).await,
        (Some("set-plan"), Some(slug), Some(plan), None) => set_plan(db, slug, plan).await,
        (Some("suspend"), Some(slug), None, _) => set_status(db, slug, "suspended").await,
        (Some("unsuspend"), Some(slug), None, _) => set_status(db, slug, "active").await,
        _ => bail!("{USAGE}"),
    }
}

/// 代稱的格式與建立店家時相同:不符合的一律拒絕,不拿去查詢
fn check_slug(slug: &str) -> Result<()> {
    let b = slug.as_bytes();
    let ok = (3..=40).contains(&b.len())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && b[0] != b'-'
        && b[b.len() - 1] != b'-';
    if !ok {
        bail!("店家代稱格式不正確:{slug}");
    }
    Ok(())
}

async fn tenant_id(db: &PgPool, slug: &str) -> Result<Uuid> {
    check_slug(slug)?;
    sqlx::query_scalar("SELECT id FROM tenants WHERE slug = $1")
        .bind(slug)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("找不到店家:{slug}"))
}

fn limit_text(v: Option<i32>) -> String {
    v.map_or("不限".to_string(), |n| n.to_string())
}

#[derive(FromRow)]
struct PlanRow {
    id: String,
    name: String,
    price_cents: i32,
    max_staff: Option<i32>,
    max_services: Option<i32>,
    max_bookings_per_month: Option<i32>,
}

async fn plans(db: &PgPool) -> Result<String> {
    let rows: Vec<PlanRow> = sqlx::query_as(
        "SELECT id, name, price_cents, max_staff, max_services, max_bookings_per_month
         FROM plans ORDER BY price_cents",
    )
    .fetch_all(db)
    .await?;
    let mut out = String::from("方案\t名稱\t月費\t成員\t服務\t每月預約\n");
    for p in rows {
        out += &format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            p.id,
            p.name,
            p.price_cents / 100,
            limit_text(p.max_staff),
            limit_text(p.max_services),
            limit_text(p.max_bookings_per_month)
        );
    }
    Ok(out)
}

#[derive(FromRow)]
struct ShopRow {
    slug: String,
    name: String,
    plan_id: String,
    status: String,
    members: i64,
    recent_bookings: i64,
    created: String,
    deleting: Option<String>,
}

async fn shops(db: &PgPool, limit: i64) -> Result<String> {
    let rows: Vec<ShopRow> = sqlx::query_as(
        "SELECT t.slug, t.name, t.plan_id, t.status,
                (SELECT count(*) FROM memberships m WHERE m.tenant_id = t.id AND m.active) AS members,
                (SELECT count(*) FROM bookings b WHERE b.tenant_id = t.id
                   AND b.status IN ('confirmed', 'completed', 'no_show')
                   AND b.created_at > now() - interval '30 days') AS recent_bookings,
                to_char(t.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD') AS created,
                to_char(t.deletion_scheduled_at AT TIME ZONE 'UTC', 'YYYY-MM-DD') AS deleting
         FROM tenants t ORDER BY t.created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(db)
    .await?;
    let mut out = String::from("代稱\t名稱\t方案\t狀態\t成員\t近30天預約\t建立日(UTC)\t預定刪除\n");
    for r in &rows {
        out += &format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            r.slug,
            r.name,
            r.plan_id,
            r.status,
            r.members,
            r.recent_bookings,
            r.created,
            r.deleting.as_deref().unwrap_or("-")
        );
    }
    out += &format!("共 {} 家(最多顯示 {limit} 家)\n", rows.len());
    Ok(out)
}

#[derive(FromRow)]
struct ShopDetail {
    name: String,
    timezone: String,
    plan_id: String,
    status: String,
    created: String,
    deleting: Option<String>,
}

async fn show(db: &PgPool, slug: &str) -> Result<String> {
    let id = tenant_id(db, slug).await?;
    let d: ShopDetail = sqlx::query_as(
        "SELECT name, timezone, plan_id, status,
                to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI') AS created,
                to_char(deletion_scheduled_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI') AS deleting
         FROM tenants WHERE id = $1",
    )
    .bind(id)
    .fetch_one(db)
    .await?;
    let owners: Vec<String> = sqlx::query_scalar(
        "SELECT u.email::text FROM memberships m JOIN users u ON u.id = m.user_id
         WHERE m.tenant_id = $1 AND m.role = 'owner' AND m.active ORDER BY u.email",
    )
    .bind(id)
    .fetch_all(db)
    .await?;
    let (members, total_bookings, month_bookings): (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM memberships WHERE tenant_id = $1 AND active),
                (SELECT count(*) FROM bookings WHERE tenant_id = $1),
                (SELECT count(*) FROM bookings WHERE tenant_id = $1
                   AND status IN ('confirmed', 'completed', 'no_show')
                   AND created_at > now() - interval '30 days')",
    )
    .bind(id)
    .fetch_one(db)
    .await?;
    let billing: Option<(Option<String>, bool)> = sqlx::query_as(
        "SELECT status, cancel_at_period_end FROM tenant_billing WHERE tenant_id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    let mut out = format!(
        "{}({slug})\n  方案:{}  狀態:{}  時區:{}\n  建立:{} UTC\n  店主:{}\n  成員:{members}  預約:共 {total_bookings} 筆,近 30 天新增 {month_bookings} 筆\n",
        d.name,
        d.plan_id,
        d.status,
        d.timezone,
        d.created,
        owners.join("、")
    );
    match billing {
        Some((st, cancel)) => {
            out += &format!(
                "  訂閱:{}{}\n",
                st.as_deref().unwrap_or("(無)"),
                if cancel { "(期末取消)" } else { "" }
            );
        }
        None => out += "  訂閱:沒有綁定 Stripe\n",
    }
    if let Some(when) = d.deleting {
        out += &format!("  已申請刪除,預定 {when} UTC 永久刪除\n");
    }
    Ok(out)
}

async fn set_plan(db: &PgPool, slug: &str, plan: &str) -> Result<String> {
    let id = tenant_id(db, slug).await?;
    let mut tx = db.begin().await?;
    let known: Option<String> = sqlx::query_scalar("SELECT id FROM plans WHERE id = $1")
        .bind(plan)
        .fetch_optional(&mut *tx)
        .await?;
    if known.is_none() {
        let all: Vec<String> = sqlx::query_scalar("SELECT id FROM plans ORDER BY price_cents")
            .fetch_all(&mut *tx)
            .await?;
        bail!("沒有這個方案:{plan}(可用:{})", all.join("、"));
    }
    let old: String = sqlx::query_scalar("SELECT plan_id FROM tenants WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if old == plan {
        return Ok(format!("{slug} 已經是 {plan},沒有變更\n"));
    }
    sqlx::query("UPDATE tenants SET plan_id = $2 WHERE id = $1")
        .bind(id)
        .bind(plan)
        .execute(&mut *tx)
        .await?;
    audit(
        &mut tx,
        id,
        "operator.plan_changed",
        json!({ "from": old, "to": plan }),
    )
    .await?;
    tx.commit().await?;
    Ok(format!("{slug}:{old} → {plan}\n"))
}

async fn set_status(db: &PgPool, slug: &str, status: &str) -> Result<String> {
    let id = tenant_id(db, slug).await?;
    let mut tx = db.begin().await?;
    let old: String = sqlx::query_scalar("SELECT status FROM tenants WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if old == status {
        return Ok(format!("{slug} 已經是 {status},沒有變更\n"));
    }
    sqlx::query("UPDATE tenants SET status = $2 WHERE id = $1")
        .bind(id)
        .bind(status)
        .execute(&mut *tx)
        .await?;
    let action = if status == "suspended" {
        "operator.suspended"
    } else {
        "operator.unsuspended"
    };
    audit(&mut tx, id, action, json!({})).await?;
    tx.commit().await?;
    Ok(format!("{slug}:{old} → {status}\n"))
}

async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    action: &str,
    detail: serde_json::Value,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_logs (tenant_id, actor_type, actor_user_id, action, entity_type, entity_id, detail)
         VALUES ($1, 'system', NULL, $2, 'tenant', $1, $3)",
    )
    .bind(tenant_id)
    .bind(action)
    .bind(detail)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[derive(FromRow)]
struct AuditExportRow {
    id: i64,
    tenant_id: Uuid,
    tenant_slug: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    actor_type: String,
    actor_user_id: Option<Uuid>,
    action: String,
    entity_type: String,
    entity_id: Option<Uuid>,
    detail: serde_json::Value,
}

/// 把超過 `days` 天的稽核日誌逐筆輸出成 JSON Lines(一行一筆,依 id 由舊到新)。
/// 不用 CSV:歸檔要**原樣**保存,CSV 為了防公式注入會改動欄位開頭;JSON Lines 無損、好壓縮、好用工具處理。
/// 用 keyset 分頁(`id > 上一批最後一個`),每批 5000 筆:記憶體用量固定,邊讀邊寫。唯讀,不會刪任何東西
/// (清除是背景任務依 `AUDIT_RETENTION_DAYS` 做的,這個指令是讓你在它動手**之前**先留一份)。
pub async fn export_audit<W: Write>(db: &PgPool, days: i32, out: &mut W) -> Result<u64> {
    let mut last = 0_i64;
    let mut total = 0_u64;
    loop {
        let rows: Vec<AuditExportRow> = sqlx::query_as(
            "SELECT a.id, a.tenant_id, t.slug AS tenant_slug, a.created_at, a.actor_type, a.actor_user_id,
                    a.action, a.entity_type, a.entity_id, a.detail
             FROM audit_logs a LEFT JOIN tenants t ON t.id = a.tenant_id
             WHERE a.id > $1 AND a.created_at < now() - make_interval(days => $2)
             ORDER BY a.id LIMIT 5000",
        )
        .bind(last)
        .bind(days)
        .fetch_all(db)
        .await?;
        let Some(tail) = rows.last() else { break };
        last = tail.id;
        for r in &rows {
            serde_json::to_writer(
                &mut *out,
                &json!({
                    "id": r.id,
                    "tenant_id": r.tenant_id,
                    "tenant_slug": r.tenant_slug,
                    "created_at": r.created_at,
                    "actor_type": r.actor_type,
                    "actor_user_id": r.actor_user_id,
                    "action": r.action,
                    "entity_type": r.entity_type,
                    "entity_id": r.entity_id,
                    "detail": r.detail,
                }),
            )?;
            out.write_all(b"\n")?;
        }
        total += rows.len() as u64;
    }
    out.flush()?;
    Ok(total)
}
