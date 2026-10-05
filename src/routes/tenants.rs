use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::AppState;
use crate::{
    auth::AuthUser,
    db::{PG_UNIQUE_VIOLATION, begin_scoped},
    error::AppError,
    plan,
    tenancy::{Role, TenantCtx},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/tenants", post(create_tenant).get(list_tenants))
        .route("/t/{slug}", patch(update_tenant))
        .route("/t/{slug}/me", get(tenant_me))
        .route("/t/{slug}/members", get(list_members))
        .route("/t/{slug}/plan", get(get_plan))
        .route("/plans", get(list_plans))
}

const RESERVED_SLUGS: &[&str] = &[
    "www", "api", "app", "admin", "static", "assets", "mail", "public",
];
const PG_INVALID_PARAMETER: &str = "22023";

#[derive(Debug, Deserialize)]
struct CreateTenantRequest {
    slug: String,
    name: String,
    timezone: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
struct TenantResponse {
    id: Uuid,
    slug: String,
    name: String,
    timezone: String,
    role: Role,
}

fn validate_slug(slug: &str) -> Result<(), AppError> {
    let bytes = slug.as_bytes();
    let ok = (3..=40).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes[0] != b'-'
        && bytes[bytes.len() - 1] != b'-';
    if !ok {
        return Err(AppError::BadRequest(
            "網址代稱需為 3–40 字,只能用小寫英數與連字號,且不可以連字號開頭或結尾".into(),
        ));
    }
    if RESERVED_SLUGS.contains(&slug) {
        return Err(AppError::BadRequest("此網址代稱為保留字".into()));
    }
    Ok(())
}

async fn create_tenant(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateTenantRequest>,
) -> Result<(StatusCode, Json<TenantResponse>), AppError> {
    let slug = req.slug.trim().to_string();
    validate_slug(&slug)?;
    let name = req.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(AppError::BadRequest("店家名稱長度需為 1–100 字".into()));
    }
    let timezone = req.timezone.unwrap_or_else(|| "Asia/Taipei".into());
    // 排程計算用 IANA 時區資料庫,必須是程式也認得的名稱,否則之後每次算時段都會失敗
    if timezone.parse::<chrono_tz::Tz>().is_err() {
        return Err(AppError::BadRequest("不支援的時區".into()));
    }

    let mut tx = begin_scoped(&state.db, Some(auth.id), None).await?;
    let result: Result<Uuid, sqlx::Error> = sqlx::query_scalar("SELECT create_tenant($1, $2, $3)")
        .bind(&slug)
        .bind(&name)
        .bind(&timezone)
        .fetch_one(&mut *tx)
        .await;
    let id = match result {
        Ok(id) => id,
        Err(sqlx::Error::Database(e)) => match e.code().as_deref() {
            Some(PG_UNIQUE_VIOLATION) => {
                return Err(AppError::Conflict("此網址代稱已被使用".into()));
            }
            Some(PG_INVALID_PARAMETER) => {
                return Err(AppError::BadRequest("不支援的時區".into()));
            }
            _ => return Err(sqlx::Error::Database(e).into()),
        },
        Err(e) => return Err(e.into()),
    };
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(TenantResponse {
            id,
            slug,
            name,
            timezone,
            role: Role::Owner,
        }),
    ))
}

async fn list_tenants(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<TenantResponse>>, AppError> {
    let mut tx = begin_scoped(&state.db, Some(auth.id), None).await?;
    let rows = sqlx::query_as::<_, TenantResponse>(
        "SELECT t.id, t.slug, t.name, t.timezone, m.role
         FROM memberships m JOIN tenants t ON t.id = m.tenant_id
         WHERE m.user_id = $1 AND m.active AND t.status = 'active'
         ORDER BY t.name",
    )
    .bind(auth.id)
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    Ok(Json(rows))
}

#[derive(Debug, Serialize, FromRow)]
struct TenantMeResponse {
    id: Uuid,
    slug: String,
    name: String,
    timezone: String,
    role: Role,
}

async fn tenant_me(
    State(state): State<AppState>,
    ctx: TenantCtx,
) -> Result<Json<TenantMeResponse>, AppError> {
    let mut tx = ctx.begin(&state).await?;
    let row = sqlx::query_as::<_, TenantMeResponse>(
        "SELECT id, slug, name, timezone, $1::member_role AS role FROM tenants WHERE id = $2",
    )
    .bind(ctx.role)
    .bind(ctx.tenant_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.rollback().await?;
    Ok(Json(row))
}

#[derive(Debug, Serialize, FromRow)]
struct MemberResponse {
    user_id: Uuid,
    name: String,
    email: String,
    role: Role,
    /// false = 已停用(離職):不能再存取這間店、不能被預約,歷史紀錄保留
    active: bool,
}

async fn list_members(
    State(state): State<AppState>,
    ctx: TenantCtx,
) -> Result<Json<Vec<MemberResponse>>, AppError> {
    let mut tx = ctx.begin(&state).await?;
    // 不手動加 WHERE tenant_id:租戶範圍完全由 RLS 決定
    let rows = sqlx::query_as::<_, MemberResponse>(
        "SELECT u.id AS user_id, u.name, u.email::text AS email, m.role, m.active
         FROM memberships m JOIN users u ON u.id = m.user_id
         ORDER BY m.active DESC, u.name",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    Ok(Json(rows))
}

#[derive(Debug, Serialize)]
struct PlanResponse {
    plan: plan::Plan,
    usage: plan::Usage,
}

/// 目前方案與用量(owner / manager)
async fn get_plan(
    State(state): State<AppState>,
    ctx: TenantCtx,
) -> Result<Json<PlanResponse>, AppError> {
    ctx.require_manager()?;
    let mut tx = ctx.begin(&state).await?;
    let tz = crate::booking::tenant_timezone(&mut tx, ctx.tenant_id).await?;
    let response = PlanResponse {
        plan: plan::current(&mut tx, ctx.tenant_id).await?,
        usage: plan::usage(&mut tx, tz).await?,
    };
    tx.rollback().await?;
    Ok(Json(response))
}

/// 公開的方案列表(定價頁用)
async fn list_plans(State(state): State<AppState>) -> Result<Json<Vec<plan::Plan>>, AppError> {
    let rows = sqlx::query_as::<_, plan::Plan>(
        "SELECT id, name, price_cents, max_staff, max_services, max_bookings_per_month
         FROM plans ORDER BY price_cents",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
struct UpdateTenantRequest {
    name: Option<String>,
    timezone: Option<String>,
}

/// 修改店家名稱 / 時區(僅店主)。網址代稱不可改:顧客手上的預約連結與書籤都靠它。
async fn update_tenant(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Json(req): Json<UpdateTenantRequest>,
) -> Result<Json<TenantMeResponse>, AppError> {
    if ctx.role != Role::Owner {
        return Err(AppError::Forbidden);
    }
    let name = req.name.map(|n| n.trim().to_string());
    if name
        .as_ref()
        .is_some_and(|n| n.is_empty() || n.chars().count() > 100)
    {
        return Err(AppError::BadRequest("店家名稱長度需為 1–100 字".into()));
    }
    if req
        .timezone
        .as_ref()
        .is_some_and(|tz| tz.parse::<chrono_tz::Tz>().is_err())
    {
        return Err(AppError::BadRequest("不支援的時區".into()));
    }
    if name.is_none() && req.timezone.is_none() {
        return Err(AppError::BadRequest("沒有要修改的欄位".into()));
    }

    let mut tx = ctx.begin(&state).await?;
    sqlx::query("SELECT update_tenant($1, $2)")
        .bind(&name)
        .bind(&req.timezone)
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query_as::<_, TenantMeResponse>(
        "SELECT id, slug, name, timezone, $1::member_role AS role FROM tenants WHERE id = $2",
    )
    .bind(ctx.role)
    .bind(ctx.tenant_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(row))
}
