use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use serde_json::json;

use super::AppState;
use crate::{
    audit::{self, Actor},
    auth::AuthUser,
    db::{PG_FOREIGN_KEY_VIOLATION, PG_UNIQUE_VIOLATION, begin_scoped, pg_code},
    error::AppError,
    mail, outbox, plan,
    tenancy::{Role, TenantCtx},
    token,
    validation::normalize_email,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/t/{slug}/invitations",
            get(list_invitations).post(create_invitation),
        )
        .route("/t/{slug}/invitations/{id}", delete(revoke_invitation))
        .route("/invitations/accept", post(accept_invitation))
        .route(
            "/t/{slug}/members/{user_id}",
            patch(change_role).delete(remove_member),
        )
}

const INVITATION_TTL_DAYS: i64 = 7;
/// PostgreSQL `no_data_found`,accept_invitation 用來表示「邀請無效」
const PG_NO_DATA_FOUND: &str = "P0002";
/// `accept_invitation` 用來表示「成員人數已達方案上限」
const PG_CONFIGURATION_LIMIT_EXCEEDED: &str = "53400";

// ---------- 邀請 ----------

#[derive(Debug, Deserialize)]
struct CreateInvitation {
    email: String,
    role: Role,
}

#[derive(Debug, Serialize)]
struct InvitationCreated {
    id: Uuid,
    email: String,
    role: Role,
    expires_at: DateTime<Utc>,
    /// 原始 token,只會在建立時回傳這一次;資料庫只存雜湊
    token: String,
}

async fn create_invitation(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Json(req): Json<CreateInvitation>,
) -> Result<(StatusCode, Json<InvitationCreated>), AppError> {
    ctx.require_manager()?;
    if req.role == Role::Owner {
        return Err(AppError::BadRequest("不能邀請成為擁有者".into()));
    }
    if !ctx.role.can_manage(req.role) {
        return Err(AppError::Forbidden);
    }
    let email = normalize_email(&req.email)?;

    let mut tx = ctx.begin(&state).await?;
    let already_member = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM memberships m JOIN users u ON u.id = m.user_id WHERE u.email = $1::citext",
    )
    .bind(&email)
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    if already_member {
        return Err(AppError::Conflict("此 Email 已經是成員".into()));
    }

    // 邀請會預留一個名額;在鎖內檢查並接著寫入
    plan::ensure_staff_slot(&mut tx, ctx.tenant_id).await?;

    // 過期但未使用的舊邀請會卡住唯一索引,先清掉
    sqlx::query("DELETE FROM invitations WHERE email = $1::citext AND accepted_at IS NULL AND expires_at <= now()")
        .bind(&email)
        .execute(&mut *tx)
        .await?;

    let token = token::generate();
    let expires_at = Utc::now() + Duration::days(INVITATION_TTL_DAYS);
    let result = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO invitations (tenant_id, email, role, token_hash, expires_at)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(ctx.tenant_id)
    .bind(&email)
    .bind(req.role)
    .bind(token::hash(&token))
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await;
    let id = match result {
        Ok(id) => id,
        Err(e) if pg_code(&e).as_deref() == Some(PG_UNIQUE_VIOLATION) => {
            return Err(AppError::Conflict("此 Email 已有待處理的邀請".into()));
        }
        Err(e) => return Err(e.into()),
    };
    let shop: String = sqlx::query_scalar("SELECT name FROM tenants WHERE id = $1")
        .bind(ctx.tenant_id)
        .fetch_one(&mut *tx)
        .await?;
    let role_label = match req.role {
        Role::Manager => "管理者",
        _ => "員工",
    };
    let link = mail::invitation_link(&state.public_base_url, &token);
    let invite_mail = mail::invitation(&email, &shop, role_label, &link);
    outbox::enqueue(
        &mut tx,
        ctx.tenant_id,
        &invite_mail,
        Some(&format!("invite:{id}")),
    )
    .await?;
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "invitation.created",
        "invitation",
        Some(id),
        json!({ "role": req.role }),
    )
    .await?;
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(InvitationCreated {
            id,
            email,
            role: req.role,
            expires_at,
            token,
        }),
    ))
}

#[derive(Debug, Serialize, FromRow)]
struct InvitationItem {
    id: Uuid,
    email: String,
    role: Role,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

async fn list_invitations(
    State(state): State<AppState>,
    ctx: TenantCtx,
) -> Result<Json<Vec<InvitationItem>>, AppError> {
    ctx.require_manager()?;
    let mut tx = ctx.begin(&state).await?;
    let rows = sqlx::query_as::<_, InvitationItem>(
        "SELECT id, email::text AS email, role, expires_at, created_at FROM invitations
         WHERE accepted_at IS NULL AND expires_at > now()
         ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    Ok(Json(rows))
}

async fn revoke_invitation(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, id)): Path<(String, Uuid)>,
) -> Result<StatusCode, AppError> {
    ctx.require_manager()?;
    let mut tx = ctx.begin(&state).await?;
    let role: Option<Role> =
        sqlx::query_scalar("SELECT role FROM invitations WHERE id = $1 AND accepted_at IS NULL")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let role = role.ok_or(AppError::NotFound)?;
    if !ctx.role.can_manage(role) {
        return Err(AppError::Forbidden);
    }
    sqlx::query("DELETE FROM invitations WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "invitation.revoked",
        "invitation",
        Some(id),
        json!({ "role": role }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct AcceptInvitation {
    token: String,
}

#[derive(Debug, Serialize)]
struct Accepted {
    tenant_slug: String,
}

async fn accept_invitation(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<AcceptInvitation>,
) -> Result<Json<Accepted>, AppError> {
    let token = req.token.trim();
    if !(16..=128).contains(&token.len()) {
        return Err(AppError::NotFound);
    }
    let mut tx = begin_scoped(&state.db, Some(auth.id), None).await?;
    let result: Result<String, sqlx::Error> = sqlx::query_scalar("SELECT accept_invitation($1)")
        .bind(token::hash(token))
        .fetch_one(&mut *tx)
        .await;
    let tenant_slug = match result {
        Ok(slug) => slug,
        Err(e) if pg_code(&e).as_deref() == Some(PG_NO_DATA_FOUND) => {
            return Err(AppError::NotFound);
        }
        Err(e) if pg_code(&e).as_deref() == Some(PG_CONFIGURATION_LIMIT_EXCEEDED) => {
            return Err(AppError::LimitReached(
                "此店家的成員人數已達方案上限,請聯絡店家".into(),
            ));
        }
        Err(e) => return Err(e.into()),
    };
    tx.commit().await?;
    Ok(Json(Accepted { tenant_slug }))
}

// ---------- 成員管理 ----------

#[derive(Debug, Deserialize)]
struct ChangeRole {
    role: Role,
}

#[derive(Debug, Serialize)]
struct MemberRole {
    user_id: Uuid,
    role: Role,
}

/// 只有 owner 能調整角色;owner 本身不能被降級,也不能把別人升成 owner(轉讓所有權不在範圍內)
async fn change_role(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, user_id)): Path<(String, Uuid)>,
    Json(req): Json<ChangeRole>,
) -> Result<Json<MemberRole>, AppError> {
    if ctx.role != Role::Owner {
        return Err(AppError::Forbidden);
    }
    if req.role == Role::Owner {
        return Err(AppError::BadRequest("不能指派擁有者角色".into()));
    }
    let mut tx = ctx.begin(&state).await?;
    let current: Option<Role> =
        sqlx::query_scalar("SELECT role FROM memberships WHERE user_id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let current = current.ok_or(AppError::NotFound)?;
    if current == Role::Owner {
        return Err(AppError::Conflict("擁有者的角色不可變更".into()));
    }
    sqlx::query("UPDATE memberships SET role = $2 WHERE user_id = $1")
        .bind(user_id)
        .bind(req.role)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "member.role_changed",
        "member",
        Some(user_id),
        json!({ "from": current, "to": req.role }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(MemberRole {
        user_id,
        role: req.role,
    }))
}

/// 移除成員,或自己退出(owner 除外)。owner 永遠不能被移除,店家至少保有一位 owner。
async fn remove_member(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, user_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, AppError> {
    let mut tx = ctx.begin(&state).await?;
    let target: Option<Role> =
        sqlx::query_scalar("SELECT role FROM memberships WHERE user_id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let target = target.ok_or(AppError::NotFound)?;
    if target == Role::Owner {
        return Err(AppError::Conflict("擁有者無法被移除或退出".into()));
    }
    if user_id != ctx.user_id && !ctx.role.can_manage(target) {
        return Err(AppError::Forbidden);
    }
    let result = sqlx::query("DELETE FROM memberships WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await;
    match result {
        Ok(_) => {}
        // bookings 以外鍵參照員工,有預約紀錄的成員不能直接刪除
        Err(e) if pg_code(&e).as_deref() == Some(PG_FOREIGN_KEY_VIOLATION) => {
            return Err(AppError::Conflict("此成員有預約紀錄,無法移除".into()));
        }
        Err(e) => return Err(e.into()),
    }
    let action = if user_id == ctx.user_id {
        "member.left"
    } else {
        "member.removed"
    };
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        action,
        "member",
        Some(user_id),
        json!({ "role": target }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
