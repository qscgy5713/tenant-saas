use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, patch, post, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::AppState;
use crate::{
    auth::AuthUser,
    db::{PG_UNIQUE_VIOLATION, begin_scoped, pg_code},
    error::AppError,
    mail, outbox, plan,
    tenancy::{Role, TenantCtx},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/tenants", post(create_tenant).get(list_tenants))
        .route("/t/{slug}", patch(update_tenant))
        .route("/t/{slug}/me", get(tenant_me))
        .route("/t/{slug}/data-retention", put(set_data_retention))
        .route("/t/{slug}/profile", put(set_profile))
        .route(
            "/t/{slug}/deletion",
            post(request_deletion).delete(cancel_deletion),
        )
        .route("/t/{slug}/members", get(list_members))
        .route("/t/{slug}/plan", get(get_plan))
        .route("/plans", get(list_plans))
}

const RESERVED_SLUGS: &[&str] = &[
    "www", "api", "app", "admin", "static", "assets", "mail", "public",
];
const PG_INVALID_PARAMETER: &str = "22023";
/// `create_tenant` 用來表示「已達每位使用者可擁有的店家數量上限」
const PG_CONFIGURATION_LIMIT_EXCEEDED: &str = "53400";

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

    // 沒驗證 Email 不能開店(防止拿別人的 Email 註冊來占用網址代稱、以系統名義寄信)
    let verified: bool =
        sqlx::query_scalar("SELECT email_verified_at IS NOT NULL FROM users WHERE id = $1")
            .bind(auth.id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::Unauthorized)?;
    if state.require_verified_email && !verified {
        return Err(AppError::EmailNotVerified);
    }

    let mut tx = begin_scoped(&state.db, Some(auth.id), None).await?;
    let result: Result<Uuid, sqlx::Error> =
        sqlx::query_scalar("SELECT create_tenant($1, $2, $3, $4)")
            .bind(&slug)
            .bind(&name)
            .bind(&timezone)
            .bind(i32::try_from(state.max_shops_per_user).unwrap_or(i32::MAX))
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
            Some(PG_CONFIGURATION_LIMIT_EXCEEDED) => {
                return Err(AppError::LimitReached(format!(
                    "每位使用者最多可以建立 {} 家店,已達上限",
                    state.max_shops_per_user
                )));
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
    /// 顧客個資在最後一筆預約之後保留幾天,到期自動匿名化
    customer_retention_days: i32,
    /// 申請刪除後預定刪除的時間;沒有申請時為 null
    deletion_scheduled_at: Option<DateTime<Utc>>,
    /// 顧客預約頁上顯示的店家資訊(都是選填)
    description: Option<String>,
    address: Option<String>,
    phone: Option<String>,
}

const TENANT_ME_SQL: &str = "SELECT id, slug, name, timezone, $1::member_role AS role,
        customer_retention_days, deletion_scheduled_at, description, address, phone
        FROM tenants WHERE id = $2";

async fn tenant_me(
    State(state): State<AppState>,
    ctx: TenantCtx,
) -> Result<Json<TenantMeResponse>, AppError> {
    let mut tx = ctx.begin(&state).await?;
    let row = sqlx::query_as::<_, TenantMeResponse>(TENANT_ME_SQL)
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
    let row = sqlx::query_as::<_, TenantMeResponse>(TENANT_ME_SQL)
        .bind(ctx.role)
        .bind(ctx.tenant_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(row))
}

#[derive(Debug, Deserialize)]
struct RetentionRequest {
    customer_retention_days: i32,
}

/// 調整顧客個資的保留天數(僅店主,90–3650)。不追溯放寬:已經匿名化的不會還原
async fn set_data_retention(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Json(req): Json<RetentionRequest>,
) -> Result<Json<TenantMeResponse>, AppError> {
    ctx.require_owner()?;
    if !(90..=3650).contains(&req.customer_retention_days) {
        return Err(AppError::BadRequest(
            "保留天數需介於 90 到 3650 天(約 3 個月到 10 年)".into(),
        ));
    }
    let mut tx = ctx.begin(&state).await?;
    sqlx::query("SELECT set_customer_retention($1)")
        .bind(req.customer_retention_days)
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query_as::<_, TenantMeResponse>(TENANT_ME_SQL)
        .bind(ctx.role)
        .bind(ctx.tenant_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(row))
}

#[derive(Debug, Deserialize)]
struct DeletionRequest {
    /// 要求輸入店家的網址代稱確認,避免手滑
    confirm_slug: String,
}

/// 申請刪除店家(僅店主)。寬限期 30 天:公開預約頁立即關閉,後台照常,店主可以取消;期滿由背景任務連同所有資料刪除。
/// 還有未來 / 待確認的預約、或還有進行中的訂閱時不能申請。
async fn request_deletion(
    State(state): State<AppState>,
    ctx: TenantCtx,
    axum::extract::Path(slug): axum::extract::Path<String>,
    Json(req): Json<DeletionRequest>,
) -> Result<Json<TenantMeResponse>, AppError> {
    ctx.require_owner()?;
    if req.confirm_slug.trim() != slug {
        return Err(AppError::BadRequest("確認用的網址代稱不符".into()));
    }
    let mut tx = ctx.begin(&state).await?;
    let when: Result<DateTime<Utc>, sqlx::Error> =
        sqlx::query_scalar("SELECT request_tenant_deletion($1)")
            .bind(crate::worker::TENANT_DELETION_GRACE_DAYS)
            .fetch_one(&mut *tx)
            .await;
    let when = match when {
        Ok(w) => w,
        Err(e) => {
            return Err(match pg_code(&e).as_deref() {
                Some("P0001") => AppError::Conflict(
                    "還有未來或待確認的預約,請先取消(顧客會收到通知)再申請刪除".into(),
                ),
                Some("P0002") => {
                    AppError::Conflict("還有進行中的訂閱,請先取消訂閱再申請刪除".into())
                }
                Some("P0003") => AppError::Conflict("已經申請過刪除了".into()),
                _ => e.into(),
            });
        }
    };
    let (shop, tz): (String, String) =
        sqlx::query_as("SELECT name, timezone FROM tenants WHERE id = $1")
            .bind(ctx.tenant_id)
            .fetch_one(&mut *tx)
            .await?;
    let tz: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    // 通知所有店主(不只是申請的人):刪除是影響整家店的大事
    let owners: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT u.id, u.email::text FROM memberships m JOIN users u ON u.id = m.user_id
         WHERE m.role = 'owner' AND m.active",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (owner_id, to) in owners {
        let msg = mail::tenant_deletion_requested(
            &to,
            &shop,
            &mail::format_local(when, tz),
            &mail::settings_link(&state.public_base_url, &slug),
        );
        outbox::enqueue(
            &mut tx,
            ctx.tenant_id,
            &msg,
            // 去重鍵用使用者 id,不要放 Email:信件寄出後內文會清空,但 dedupe_key 會一直留著
            Some(&format!(
                "tenant_deletion:{}:{}:{owner_id}",
                ctx.tenant_id,
                when.timestamp()
            )),
        )
        .await?;
    }
    let row = sqlx::query_as::<_, TenantMeResponse>(TENANT_ME_SQL)
        .bind(ctx.role)
        .bind(ctx.tenant_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(row))
}

/// 取消刪除(寬限期內,僅店主)
async fn cancel_deletion(
    State(state): State<AppState>,
    ctx: TenantCtx,
) -> Result<Json<TenantMeResponse>, AppError> {
    ctx.require_owner()?;
    let mut tx = ctx.begin(&state).await?;
    let cancelled: bool = sqlx::query_scalar("SELECT cancel_tenant_deletion()")
        .fetch_one(&mut *tx)
        .await?;
    if !cancelled {
        return Err(AppError::Conflict("這家店沒有在申請刪除".into()));
    }
    let row = sqlx::query_as::<_, TenantMeResponse>(TENANT_ME_SQL)
        .bind(ctx.role)
        .bind(ctx.tenant_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(row))
}

#[derive(Debug, Deserialize)]
struct ProfileRequest {
    description: Option<String>,
    address: Option<String>,
    phone: Option<String>,
}

/// 清理一個選填的文字欄位:去頭尾空白、空字串視為清除、不得含控制字元(換行只有簡介可以有)
fn clean_profile_field(
    raw: Option<String>,
    label: &str,
    max: usize,
    allow_newlines: bool,
) -> Result<Option<String>, AppError> {
    let Some(raw) = raw else { return Ok(None) };
    let v = raw.trim().to_string();
    if v.is_empty() {
        return Ok(None);
    }
    if v.chars().count() > max {
        return Err(AppError::BadRequest(format!("{label}最多 {max} 字")));
    }
    if v.chars()
        .any(|c| c.is_control() && !(allow_newlines && (c == '\n' || c == '\r')))
    {
        return Err(AppError::BadRequest(format!("{label}含有不允許的字元")));
    }
    Ok(Some(v))
}

/// 修改顧客預約頁上的店家資訊(僅店主)。整組取代:沒帶或空字串 = 清除。
/// 電話只允許數字與 `+ - ( ) # 空白` —— 預約頁會把它做成 `tel:` 連結,不能讓任意文字進到連結裡
async fn set_profile(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Json(req): Json<ProfileRequest>,
) -> Result<Json<TenantMeResponse>, AppError> {
    ctx.require_owner()?;
    let description = clean_profile_field(req.description, "店家簡介", 500, true)?;
    let address = clean_profile_field(req.address, "地址", 200, false)?;
    let phone = clean_profile_field(req.phone, "電話", 30, false)?;
    if phone.as_ref().is_some_and(|p| {
        !p.chars()
            .all(|c| c.is_ascii_digit() || "+-()# ".contains(c))
    }) {
        return Err(AppError::BadRequest(
            "電話只能包含數字與 + - ( ) # 空白".into(),
        ));
    }
    let mut tx = ctx.begin(&state).await?;
    sqlx::query("SELECT update_shop_profile($1, $2, $3)")
        .bind(&description)
        .bind(&address)
        .bind(&phone)
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query_as::<_, TenantMeResponse>(TENANT_ME_SQL)
        .bind(ctx.role)
        .bind(ctx.tenant_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(row))
}
