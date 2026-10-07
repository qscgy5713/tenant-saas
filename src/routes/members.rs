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
    booking,
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
        .route(
            "/t/{slug}/members/{user_id}",
            patch(change_role).delete(remove_member),
        )
        .route("/t/{slug}/transfer-ownership", post(transfer_ownership))
        .route(
            "/t/{slug}/members/{user_id}/deactivate",
            post(deactivate_member),
        )
        .route(
            "/t/{slug}/members/{user_id}/reactivate",
            post(reactivate_member),
        )
}

/// 接受邀請:另外掛限流(見 routes/mod.rs),與登入 / 註冊同一個限額。
/// 需要登入而且 token 是 256 位元的隨機值,猜中不可行;限流是為了不讓它被拿來消耗資源。
pub fn accept_routes() -> Router<AppState> {
    Router::new().route("/invitations/accept", post(accept_invitation))
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
    let existing = sqlx::query_scalar::<_, bool>(
        "SELECT m.active FROM memberships m JOIN users u ON u.id = m.user_id WHERE u.email = $1::citext",
    )
    .bind(&email)
    .fetch_optional(&mut *tx)
    .await?;
    match existing {
        Some(true) => return Err(AppError::Conflict("此 Email 已經是成員".into())),
        Some(false) => {
            return Err(AppError::Conflict(
                "此成員已停用,請在團隊頁「重新啟用」,不需要再邀請".into(),
            ));
        }
        None => {}
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
    outbox::ensure_recipient_quota(
        &mut tx,
        &invite_mail.to,
        state.max_mails_per_recipient_per_hour,
    )
    .await?;
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
    let current: Option<(Role, bool)> =
        sqlx::query_as("SELECT role, active FROM memberships WHERE user_id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (current, active) = current.ok_or(AppError::NotFound)?;
    if current == Role::Owner {
        return Err(AppError::Conflict("擁有者的角色不可變更".into()));
    }
    if !active {
        return Err(AppError::Conflict("此成員已停用,請先重新啟用".into()));
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
            return Err(AppError::Conflict(
                "此成員有預約紀錄,無法移除。請改用「停用」".into(),
            ));
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

#[derive(Debug, Default, Deserialize)]
struct DeactivateBody {
    /// 把未來已確認的預約改派給這位員工(整批,任何一筆排不進去就整個不停用)
    reassign_to: Option<Uuid>,
    /// 逐筆自動分配給「那個時段有空、提供該服務」的其他在職成員(不能和 `reassign_to` 同時用)
    #[serde(default)]
    auto_reassign: bool,
}

/// 停用成員(離職):有預約紀錄的成員不能刪除,改成停用。
/// 管理者可停用比自己低階的成員,成員也可以停用自己(退出);擁有者不行。
/// 還有「未來已確認」的預約時不准停用 —— 顧客會撲空;請先取消(顧客會收到通知信)或請顧客改期,
/// 或帶 `reassign_to` 把它們整批改派給另一位員工(時間不變,顧客會收到通知信)。
/// 待確認的預約不擋:它不佔時段,而且顧客確認時會重新檢查,員工已停用就會失敗。
async fn deactivate_member(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, user_id)): Path<(String, Uuid)>,
    body: axum::body::Bytes,
) -> Result<StatusCode, AppError> {
    // 前端的 fetch 一律帶 JSON 的 Content-Type,沒有改派時 body 是空的:空 body 視為沒帶,
    // 不能用 Option<Json>(它對「有 Content-Type 但 body 為空」會直接拒絕)
    let parsed = if body.is_empty() {
        DeactivateBody::default()
    } else {
        serde_json::from_slice::<DeactivateBody>(&body)
            .map_err(|_| AppError::BadRequest("請求內容格式不正確".into()))?
    };
    if parsed.auto_reassign && parsed.reassign_to.is_some() {
        return Err(AppError::BadRequest(
            "指定改派對象與自動分配只能擇一".into(),
        ));
    }
    let reassign = match (parsed.reassign_to, parsed.auto_reassign) {
        (Some(id), _) => Some(ReassignTarget::One(id)),
        (None, true) => Some(ReassignTarget::Auto),
        (None, false) => None,
    };
    let mut tx = ctx.begin(&state).await?;
    let row: Option<(Role, bool)> =
        sqlx::query_as("SELECT role, active FROM memberships WHERE user_id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (target, active) = row.ok_or(AppError::NotFound)?;
    if target == Role::Owner {
        return Err(AppError::Conflict("擁有者無法被停用".into()));
    }
    if user_id != ctx.user_id && !ctx.role.can_manage(target) {
        return Err(AppError::Forbidden);
    }
    if !active {
        return Err(AppError::Conflict("此成員已經停用".into()));
    }
    let upcoming: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM bookings
         WHERE staff_user_id = $1 AND status = 'confirmed' AND starts_at > now()",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    let mut reassigned = 0;
    if upcoming > 0 {
        let Some(target) = reassign else {
            return Err(AppError::Conflict(format!(
                "此成員還有 {upcoming} 筆未來已確認的預約,請先取消(顧客會收到通知)、請顧客改期,或改派給其他員工"
            )));
        };
        reassigned = reassign_upcoming(&mut tx, &ctx, user_id, target).await?;
    }
    sqlx::query("UPDATE memberships SET active = false WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "member.deactivated",
        "member",
        Some(user_id),
        json!({ "role": target, "self": user_id == ctx.user_id, "reassigned": reassigned }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Clone, Copy)]
enum ReassignTarget {
    /// 全部給這一位
    One(Uuid),
    /// 逐筆挑「那個時段有空、提供該服務」的其他在職成員
    Auto,
}

/// 把 `from` 未來已確認的預約全部改派出去,回傳筆數。呼叫端已鎖住 `from` 的成員列。
async fn reassign_upcoming(
    tx: &mut crate::db::Tx,
    ctx: &TenantCtx,
    from: Uuid,
    target: ReassignTarget,
) -> Result<usize, AppError> {
    // 指定對象時先驗證:必須是這家店仍在職的成員(RLS 已限定在這家店)
    let fixed = match target {
        ReassignTarget::One(to) => {
            if to == from {
                return Err(AppError::BadRequest("不能改派給要停用的人自己".into()));
            }
            let name: Option<String> = sqlx::query_scalar(
                "SELECT u.name FROM memberships m JOIN users u ON u.id = m.user_id
                 WHERE m.user_id = $1 AND m.active",
            )
            .bind(to)
            .fetch_optional(&mut **tx)
            .await?;
            let name =
                name.ok_or_else(|| AppError::BadRequest("改派的對象不是這家店的在職成員".into()))?;
            Some((to, name))
        }
        ReassignTarget::Auto => None,
    };

    let tz = booking::tenant_timezone(tx, ctx.tenant_id).await?;
    let rows: Vec<(Uuid, Uuid, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, service_id, starts_at FROM bookings
         WHERE staff_user_id = $1 AND status = 'confirmed' AND starts_at > now()
         ORDER BY starts_at FOR UPDATE",
    )
    .bind(from)
    .fetch_all(&mut **tx)
    .await?;
    for (id, service_id, starts_at) in &rows {
        let service = match booking::active_service(tx, *service_id).await {
            Ok(s) => s,
            Err(AppError::BadRequest(_)) => {
                return Err(AppError::Conflict(format!(
                    "{} 的預約所屬的服務已停用,無法改派,請先取消",
                    mail::format_local(*starts_at, tz)
                )));
            }
            Err(e) => return Err(e),
        };
        let old = booking::mail_ctx(tx, *id).await?;
        let (to, target_name) = match &fixed {
            Some((to, name)) => (*to, name.clone()),
            None => {
                // 自動分配:這個時段有空、提供該服務的其他成員,取排在最前面的那位
                let free = booking::free_staff_for(tx, tz, &service, *starts_at, *id).await?;
                let pick = free.into_iter().find(|s| *s != from).ok_or_else(|| {
                    AppError::Conflict(format!(
                        "{} 的預約找不到有空的其他成員,無法自動分配,請手動指定或先取消",
                        old.when
                    ))
                })?;
                (pick, String::from("自動挑選的成員"))
            }
        };
        booking::reassign_booking(tx, tz, *id, &service, to, *starts_at)
            .await
            .map_err(|e| match e {
                AppError::Conflict(why) => {
                    AppError::Conflict(format!("{} 的預約無法改派給 {target_name}:{why}", old.when))
                }
                other => other,
            })?;
        audit::record(
            tx,
            ctx.tenant_id,
            Actor::User(ctx.user_id),
            "booking.reassigned",
            "booking",
            Some(*id),
            json!({ "from": from, "to": to }),
        )
        .await?;
        let now = booking::mail_ctx(tx, *id).await?;
        outbox::enqueue(
            tx,
            ctx.tenant_id,
            &mail::staff_changed(&now.view(), &old.staff),
            // 結尾加隨機值:同一筆預約可能在 30 天內(outbox 保留期)被改派回同一位員工(A → B → A → B),
            // 固定的鍵會讓第二次的通知被去重吞掉。每次改派本來就只排一封,不需要去重
            Some(&format!(
                "staff_changed:{id}:{to}:{}",
                Uuid::new_v4().simple()
            )),
        )
        .await?;
    }
    Ok(rows.len())
}

/// 重新啟用。要有名額(停用的成員不佔名額,所以回來時要重新檢查方案上限);
/// 只有管理者以上能做 —— 停用中的成員自己已經進不了這間店。
async fn reactivate_member(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path((_slug, user_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, AppError> {
    let mut tx = ctx.begin(&state).await?;
    let row: Option<(Role, bool)> =
        sqlx::query_as("SELECT role, active FROM memberships WHERE user_id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (target, active) = row.ok_or(AppError::NotFound)?;
    if !ctx.role.can_manage(target) {
        return Err(AppError::Forbidden);
    }
    if active {
        return Err(AppError::Conflict("此成員已經是啟用狀態".into()));
    }
    // 已刪除帳號(匿名化,見 migration 0030)的成員資格也是「停用」,但那個人已經不存在、也登入不了:
    // 重新啟用會讓顧客能預約到一位永遠不會出現的人,還白占一個名額
    let deleted: bool = sqlx::query_scalar(
        "SELECT email::text LIKE '%@anonymized.invalid' FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if deleted {
        return Err(AppError::Conflict(
            "這位成員已經刪除了自己的帳號,無法重新啟用".into(),
        ));
    }
    plan::ensure_staff_slot(&mut tx, ctx.tenant_id).await?;
    sqlx::query("UPDATE memberships SET active = true WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "member.reactivated",
        "member",
        Some(user_id),
        json!({ "role": target }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct TransferOwnership {
    user_id: Uuid,
    /// 要重新輸入自己的密碼:被盜用的登入狀態不能直接把店送人
    password: String,
}

/// 移交店主身分(僅店主):對方變成擁有者,自己降為管理者。對方必須是這家店在職的成員。
/// 要重新輸入密碼(錯誤回 400 而不是 401:前端對 401 一律當成登入過期)。店家永遠只有一位擁有者。
async fn transfer_ownership(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path(slug): Path<String>,
    Json(req): Json<TransferOwnership>,
) -> Result<StatusCode, AppError> {
    ctx.require_owner()?;
    if req.user_id == ctx.user_id {
        return Err(AppError::BadRequest("不能移交給自己".into()));
    }
    // 失敗計入帳號鎖定(和登入共用),否則持有被盜登入狀態的人可以無限次猜密碼
    super::auth::confirm_password(&state, ctx.user_id, req.password).await?;

    let mut tx = ctx.begin(&state).await?;
    let rows: Vec<(Uuid, Role, bool)> = sqlx::query_as(
        "SELECT user_id, role, active FROM memberships WHERE user_id = ANY($1) ORDER BY user_id FOR UPDATE",
    )
    .bind(vec![ctx.user_id, req.user_id])
    .fetch_all(&mut *tx)
    .await?;
    let target = rows
        .iter()
        .find(|(id, _, _)| *id == req.user_id)
        .ok_or(AppError::NotFound)?;
    if !target.2 {
        return Err(AppError::Conflict("對方已停用,請先重新啟用".into()));
    }
    if target.1 == Role::Owner {
        return Err(AppError::Conflict("對方已經是擁有者".into()));
    }
    // 自己必須(仍)是擁有者:鎖定之後再確認一次,避免兩個移交同時進行
    if !rows
        .iter()
        .any(|(id, role, _)| *id == ctx.user_id && *role == Role::Owner)
    {
        return Err(AppError::Forbidden);
    }
    sqlx::query("UPDATE memberships SET role = 'owner' WHERE user_id = $1")
        .bind(req.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE memberships SET role = 'manager' WHERE user_id = $1")
        .bind(ctx.user_id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "ownership.transferred",
        "member",
        Some(req.user_id),
        json!({ "from": ctx.user_id, "to": req.user_id }),
    )
    .await?;
    // 通知新店主(寄給資料庫裡的 Email)
    let (to, shop, old_name): (String, String, String) = sqlx::query_as(
        "SELECT (SELECT email::text FROM users WHERE id = $1), t.name,
                (SELECT name FROM users WHERE id = $2)
         FROM tenants t WHERE t.id = $3",
    )
    .bind(req.user_id)
    .bind(ctx.user_id)
    .bind(ctx.tenant_id)
    .fetch_one(&mut *tx)
    .await?;
    outbox::enqueue(
        &mut tx,
        ctx.tenant_id,
        &mail::ownership_transferred(
            &to,
            &shop,
            &old_name,
            &mail::settings_link(&state.public_base_url, &slug),
        ),
        // 結尾加隨機值:A → B → A → B 這樣來回移交時,固定的鍵會讓之後的通知被去重吞掉(見 staff_changed)
        Some(&format!(
            "ownership_transferred:{}:{}:{}:{}",
            ctx.tenant_id,
            ctx.user_id,
            req.user_id,
            Uuid::new_v4().simple()
        )),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
