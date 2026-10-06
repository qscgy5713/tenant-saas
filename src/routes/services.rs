use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use serde_json::json;

use super::AppState;
use crate::{
    audit::{self, Actor},
    db::{PG_FOREIGN_KEY_VIOLATION, pg_code},
    error::AppError,
    plan,
    tenancy::TenantCtx,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/t/{slug}/services", get(list).post(create))
        .route(
            "/t/{slug}/services/{id}",
            get(get_one).patch(update).delete(remove),
        )
}

#[derive(Debug, Serialize, FromRow)]
struct Service {
    id: Uuid,
    name: String,
    duration_minutes: i32,
    /// 服務結束後員工的整理時間(分鐘)。顧客看不到,只影響員工的下一筆預約最早何時開始
    buffer_minutes: i32,
    price_cents: i32,
    active: bool,
}

fn check_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(AppError::BadRequest("服務名稱長度需為 1–100 字".into()));
    }
    Ok(name.to_string())
}

fn check_duration(minutes: i32) -> Result<(), AppError> {
    if (5..=1440).contains(&minutes) {
        Ok(())
    } else {
        Err(AppError::BadRequest("服務時長需為 5–1440 分鐘".into()))
    }
}

fn check_buffer(minutes: i32) -> Result<(), AppError> {
    if (0..=120).contains(&minutes) {
        Ok(())
    } else {
        Err(AppError::BadRequest("整理時間需為 0–120 分鐘".into()))
    }
}

fn check_price(cents: i32) -> Result<(), AppError> {
    if (0..=10_000_000).contains(&cents) {
        Ok(())
    } else {
        Err(AppError::BadRequest("價格超出範圍".into()))
    }
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    active: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Serialize)]
struct Page<T> {
    items: Vec<T>,
    total: i64,
    limit: i64,
    offset: i64,
}

async fn list(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Query(q): Query<ListQuery>,
) -> Result<Json<Page<Service>>, AppError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 100);
    let offset = q.offset.unwrap_or(0).max(0);

    let mut tx = ctx.begin(&state).await?;
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM services WHERE ($1::boolean IS NULL OR active = $1)",
    )
    .bind(q.active)
    .fetch_one(&mut *tx)
    .await?;
    let items = sqlx::query_as::<_, Service>(
        "SELECT id, name, duration_minutes, buffer_minutes, price_cents, active FROM services
         WHERE ($1::boolean IS NULL OR active = $1)
         ORDER BY name, id LIMIT $2 OFFSET $3",
    )
    .bind(q.active)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    Ok(Json(Page {
        items,
        total,
        limit,
        offset,
    }))
}

#[derive(Debug, Deserialize)]
struct CreateService {
    name: String,
    duration_minutes: i32,
    /// 沒給就是 0(沒有整理時間)
    buffer_minutes: Option<i32>,
    price_cents: Option<i32>,
}

async fn create(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Json(req): Json<CreateService>,
) -> Result<(StatusCode, Json<Service>), AppError> {
    ctx.require_manager()?;
    let name = check_name(&req.name)?;
    check_duration(req.duration_minutes)?;
    let buffer = req.buffer_minutes.unwrap_or(0);
    check_buffer(buffer)?;
    let price = req.price_cents.unwrap_or(0);
    check_price(price)?;

    let mut tx = ctx.begin(&state).await?;
    plan::ensure_service_slot(&mut tx, ctx.tenant_id).await?;
    let service = sqlx::query_as::<_, Service>(
        "INSERT INTO services (tenant_id, name, duration_minutes, buffer_minutes, price_cents)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, name, duration_minutes, buffer_minutes, price_cents, active",
    )
    .bind(ctx.tenant_id)
    .bind(&name)
    .bind(req.duration_minutes)
    .bind(buffer)
    .bind(price)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "service.created",
        "service",
        Some(service.id),
        json!({
            "duration_minutes": service.duration_minutes,
            "buffer_minutes": service.buffer_minutes,
            "price_cents": service.price_cents
        }),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(service)))
}

async fn get_one(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, id)): Path<(String, Uuid)>,
) -> Result<Json<Service>, AppError> {
    let mut tx = ctx.begin(&state).await?;
    let service = sqlx::query_as::<_, Service>(
        "SELECT id, name, duration_minutes, buffer_minutes, price_cents, active FROM services WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.rollback().await?;
    service.map(Json).ok_or(AppError::NotFound)
}

#[derive(Debug, Deserialize)]
struct UpdateService {
    name: Option<String>,
    duration_minutes: Option<i32>,
    buffer_minutes: Option<i32>,
    price_cents: Option<i32>,
    active: Option<bool>,
}

async fn update(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, id)): Path<(String, Uuid)>,
    Json(req): Json<UpdateService>,
) -> Result<Json<Service>, AppError> {
    ctx.require_manager()?;
    let name = req.name.as_deref().map(check_name).transpose()?;
    if let Some(m) = req.duration_minutes {
        check_duration(m)?;
    }
    if let Some(m) = req.buffer_minutes {
        check_buffer(m)?;
    }
    if let Some(p) = req.price_cents {
        check_price(p)?;
    }

    let mut tx = ctx.begin(&state).await?;
    // 重新啟用已停用的服務也會增加「啟用中」的數量,不檢查的話停用再啟用就能繞過上限
    if req.active == Some(true) {
        let was_active: Option<bool> =
            sqlx::query_scalar("SELECT active FROM services WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        if was_active == Some(false) {
            plan::ensure_service_slot(&mut tx, ctx.tenant_id).await?;
        }
    }
    // 先記下要改哪些欄位(只記欄位名稱與非敏感的新值)
    let mut changed = serde_json::Map::new();
    if let Some(n) = &name {
        changed.insert("name".into(), json!(n));
    }
    if let Some(v) = req.duration_minutes {
        changed.insert("duration_minutes".into(), json!(v));
    }
    if let Some(v) = req.buffer_minutes {
        changed.insert("buffer_minutes".into(), json!(v));
    }
    if let Some(v) = req.price_cents {
        changed.insert("price_cents".into(), json!(v));
    }
    if let Some(v) = req.active {
        changed.insert("active".into(), json!(v));
    }
    let service = sqlx::query_as::<_, Service>(
        "UPDATE services SET
            name = COALESCE($2, name),
            duration_minutes = COALESCE($3, duration_minutes),
            buffer_minutes = COALESCE($4, buffer_minutes),
            price_cents = COALESCE($5, price_cents),
            active = COALESCE($6, active)
         WHERE id = $1
         RETURNING id, name, duration_minutes, buffer_minutes, price_cents, active",
    )
    .bind(id)
    .bind(name)
    .bind(req.duration_minutes)
    .bind(req.buffer_minutes)
    .bind(req.price_cents)
    .bind(req.active)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(service) = service else {
        return Err(AppError::NotFound);
    };
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "service.updated",
        "service",
        Some(service.id),
        json!({ "changed": changed }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(service))
}

async fn remove(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, id)): Path<(String, Uuid)>,
) -> Result<StatusCode, AppError> {
    ctx.require_manager()?;
    let mut tx = ctx.begin(&state).await?;
    let result = sqlx::query("DELETE FROM services WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await;
    let affected = match result {
        Ok(r) => r.rows_affected(),
        // 之後已有預約參照的服務不能刪除,請改為停用
        Err(e) if pg_code(&e).as_deref() == Some(PG_FOREIGN_KEY_VIOLATION) => {
            return Err(AppError::Conflict("此服務已有預約,請改為停用".into()));
        }
        Err(e) => return Err(e.into()),
    };
    if affected == 0 {
        return Err(AppError::NotFound);
    }
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "service.deleted",
        "service",
        Some(id),
        audit::empty(),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
