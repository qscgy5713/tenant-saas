use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get},
};
use chrono::{DateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use serde_json::json;

use super::AppState;
use crate::{
    audit::{self, Actor},
    db::{PG_EXCLUSION_VIOLATION, PG_FOREIGN_KEY_VIOLATION, Tx, pg_code},
    error::AppError,
    tenancy::TenantCtx,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/t/{slug}/members/{user_id}/services",
            get(get_staff_services).put(put_staff_services),
        )
        .route(
            "/t/{slug}/members/{user_id}/working-hours",
            get(get_working_hours).put(put_working_hours),
        )
        .route(
            "/t/{slug}/members/{user_id}/time-off",
            get(list_time_off).post(create_time_off),
        )
        .route("/t/{slug}/time-off/{id}", delete(delete_time_off))
}

/// 目標使用者必須是目前店家的成員(RLS 已限定租戶),否則回 404
async fn ensure_member(tx: &mut Tx, user_id: Uuid) -> Result<(), AppError> {
    sqlx::query_scalar::<_, i32>("SELECT 1 FROM memberships WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await?
        .map(|_| ())
        .ok_or(AppError::NotFound)
}

// ---------- 員工可提供的服務 ----------

#[derive(Debug, Serialize, Deserialize)]
struct ServiceIds {
    service_ids: Vec<Uuid>,
}

async fn load_staff_services(tx: &mut Tx, user_id: Uuid) -> Result<ServiceIds, AppError> {
    let service_ids = sqlx::query_scalar(
        "SELECT service_id FROM staff_services WHERE user_id = $1 ORDER BY service_id",
    )
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(ServiceIds { service_ids })
}

async fn get_staff_services(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, user_id)): Path<(String, Uuid)>,
) -> Result<Json<ServiceIds>, AppError> {
    let mut tx = ctx.begin(&state).await?;
    ensure_member(&mut tx, user_id).await?;
    let ids = load_staff_services(&mut tx, user_id).await?;
    tx.rollback().await?;
    Ok(Json(ids))
}

async fn put_staff_services(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, user_id)): Path<(String, Uuid)>,
    Json(req): Json<ServiceIds>,
) -> Result<Json<ServiceIds>, AppError> {
    ctx.require_manager()?;
    if req.service_ids.len() > 200 {
        return Err(AppError::BadRequest("服務數量過多".into()));
    }
    let mut tx = ctx.begin(&state).await?;
    ensure_member(&mut tx, user_id).await?;
    sqlx::query("DELETE FROM staff_services WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for service_id in &req.service_ids {
        let result = sqlx::query(
            "INSERT INTO staff_services (tenant_id, user_id, service_id) VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING",
        )
        .bind(ctx.tenant_id)
        .bind(user_id)
        .bind(service_id)
        .execute(&mut *tx)
        .await;
        match result {
            Ok(_) => {}
            // 複合外鍵 (tenant_id, service_id):別家店或不存在的服務都會失敗
            Err(e) if pg_code(&e).as_deref() == Some(PG_FOREIGN_KEY_VIOLATION) => {
                return Err(AppError::BadRequest("包含不存在的服務".into()));
            }
            Err(e) => return Err(e.into()),
        }
    }
    let ids = load_staff_services(&mut tx, user_id).await?;
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "member.services_replaced",
        "member",
        Some(user_id),
        json!({ "count": ids.service_ids.len() }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(ids))
}

// ---------- 每週營業時間 ----------

#[derive(Debug, Serialize, Deserialize, Clone)]
struct HoursEntry {
    /// 0 = 週日 … 6 = 週六
    weekday: i16,
    /// "HH:MM"
    start: String,
    end: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct HoursBody {
    hours: Vec<HoursEntry>,
}

fn parse_time(s: &str) -> Result<NaiveTime, AppError> {
    NaiveTime::parse_from_str(s, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(s, "%H:%M:%S"))
        .map_err(|_| AppError::BadRequest(format!("時間格式需為 HH:MM: {s}")))
}

#[derive(FromRow)]
struct HoursRow {
    weekday: i16,
    start_time: NaiveTime,
    end_time: NaiveTime,
}

async fn load_working_hours(tx: &mut Tx, user_id: Uuid) -> Result<HoursBody, AppError> {
    let rows = sqlx::query_as::<_, HoursRow>(
        "SELECT weekday, start_time, end_time FROM working_hours
         WHERE user_id = $1 ORDER BY weekday, start_time",
    )
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await?;
    let hours = rows
        .into_iter()
        .map(|r| HoursEntry {
            weekday: r.weekday,
            start: r.start_time.format("%H:%M").to_string(),
            end: r.end_time.format("%H:%M").to_string(),
        })
        .collect();
    Ok(HoursBody { hours })
}

async fn get_working_hours(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, user_id)): Path<(String, Uuid)>,
) -> Result<Json<HoursBody>, AppError> {
    let mut tx = ctx.begin(&state).await?;
    ensure_member(&mut tx, user_id).await?;
    let body = load_working_hours(&mut tx, user_id).await?;
    tx.rollback().await?;
    Ok(Json(body))
}

async fn put_working_hours(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, user_id)): Path<(String, Uuid)>,
    Json(req): Json<HoursBody>,
) -> Result<Json<HoursBody>, AppError> {
    ctx.require_manager_or_self(user_id)?;
    if req.hours.len() > 50 {
        return Err(AppError::BadRequest("營業時段過多".into()));
    }
    let mut parsed = Vec::with_capacity(req.hours.len());
    for h in &req.hours {
        if !(0..=6).contains(&h.weekday) {
            return Err(AppError::BadRequest(
                "weekday 需為 0(週日)到 6(週六)".into(),
            ));
        }
        let (start, end) = (parse_time(&h.start)?, parse_time(&h.end)?);
        if end <= start {
            return Err(AppError::BadRequest("結束時間必須晚於開始時間".into()));
        }
        parsed.push((h.weekday, start, end));
    }

    let mut tx = ctx.begin(&state).await?;
    ensure_member(&mut tx, user_id).await?;
    sqlx::query("DELETE FROM working_hours WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for (weekday, start, end) in parsed {
        let result = sqlx::query(
            "INSERT INTO working_hours (tenant_id, user_id, weekday, start_time, end_time)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(ctx.tenant_id)
        .bind(user_id)
        .bind(weekday)
        .bind(start)
        .bind(end)
        .execute(&mut *tx)
        .await;
        match result {
            Ok(_) => {}
            Err(e) if pg_code(&e).as_deref() == Some(PG_EXCLUSION_VIOLATION) => {
                return Err(AppError::Conflict("同一天的營業時段不可重疊".into()));
            }
            Err(e) => return Err(e.into()),
        }
    }
    let body = load_working_hours(&mut tx, user_id).await?;
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "member.working_hours_replaced",
        "member",
        Some(user_id),
        json!({ "entries": body.hours.len() }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(body))
}

// ---------- 休假 / 不可預約時段 ----------

#[derive(Debug, Serialize, FromRow)]
struct TimeOff {
    id: Uuid,
    user_id: Uuid,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    reason: Option<String>,
}

async fn list_time_off(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, user_id)): Path<(String, Uuid)>,
) -> Result<Json<Vec<TimeOff>>, AppError> {
    let mut tx = ctx.begin(&state).await?;
    ensure_member(&mut tx, user_id).await?;
    let rows = sqlx::query_as::<_, TimeOff>(
        "SELECT id, user_id, starts_at, ends_at, reason FROM time_off
         WHERE user_id = $1 ORDER BY starts_at LIMIT 500",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    // 休假原因可能涉及隱私(例如就醫),只有主管與本人看得到
    let mut rows = rows;
    if ctx.require_manager_or_self(user_id).is_err() {
        for row in &mut rows {
            row.reason = None;
        }
    }
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
struct CreateTimeOff {
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    reason: Option<String>,
}

async fn create_time_off(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, user_id)): Path<(String, Uuid)>,
    Json(req): Json<CreateTimeOff>,
) -> Result<(StatusCode, Json<TimeOff>), AppError> {
    ctx.require_manager_or_self(user_id)?;
    if req.ends_at <= req.starts_at {
        return Err(AppError::BadRequest("結束時間必須晚於開始時間".into()));
    }
    if req.reason.as_ref().is_some_and(|r| r.chars().count() > 200) {
        return Err(AppError::BadRequest("原因最多 200 字".into()));
    }
    let mut tx = ctx.begin(&state).await?;
    ensure_member(&mut tx, user_id).await?;
    let row = sqlx::query_as::<_, TimeOff>(
        "INSERT INTO time_off (tenant_id, user_id, starts_at, ends_at, reason)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, user_id, starts_at, ends_at, reason",
    )
    .bind(ctx.tenant_id)
    .bind(user_id)
    .bind(req.starts_at)
    .bind(req.ends_at)
    .bind(req.reason)
    .fetch_one(&mut *tx)
    .await?;
    // 休假原因可能涉及隱私,不寫進(無法刪除的)稽核紀錄
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "time_off.created",
        "time_off",
        Some(row.id),
        json!({ "member_id": user_id }),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

async fn delete_time_off(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, id)): Path<(String, Uuid)>,
) -> Result<StatusCode, AppError> {
    let mut tx = ctx.begin(&state).await?;
    let owner: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM time_off WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let owner = owner.ok_or(AppError::NotFound)?;
    ctx.require_manager_or_self(owner)?;
    sqlx::query("DELETE FROM time_off WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "time_off.deleted",
        "time_off",
        Some(id),
        json!({ "member_id": owner }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
