use std::collections::HashMap;

use axum::{
    extract::{FromRequestParts, Path},
    http::request::Parts,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    db::{Tx, begin_scoped},
    error::AppError,
    routes::AppState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "member_role", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Manager,
    Staff,
}

impl Role {
    /// 我能不能邀請 / 移除這個角色的人:owner 管 manager 與 staff,manager 只管 staff,staff 不管任何人。
    /// owner 永遠不能被別人管理。
    pub fn can_manage(self, target: Role) -> bool {
        matches!(
            (self, target),
            (Role::Owner, Role::Manager | Role::Staff) | (Role::Manager, Role::Staff)
        )
    }
}

/// 已驗證「這個使用者確實是該店家成員」的請求上下文。
/// 路由形如 `/t/{slug}/...`;不是成員與店家不存在一律回 404,不洩漏店家是否存在。
#[derive(Debug, Clone)]
pub struct TenantCtx {
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub role: Role,
}

impl TenantCtx {
    /// owner 或 manager 才能做的操作
    pub fn require_manager(&self) -> Result<(), AppError> {
        match self.role {
            Role::Owner | Role::Manager => Ok(()),
            Role::Staff => Err(AppError::Forbidden),
        }
    }

    /// 只有擁有者(不可逆或影響整家店的操作)
    pub fn require_owner(&self) -> Result<(), AppError> {
        match self.role {
            Role::Owner => Ok(()),
            Role::Manager | Role::Staff => Err(AppError::Forbidden),
        }
    }

    /// owner / manager,或是操作自己的資料
    pub fn require_manager_or_self(&self, target: Uuid) -> Result<(), AppError> {
        if target == self.user_id {
            Ok(())
        } else {
            self.require_manager()
        }
    }

    pub async fn begin(&self, state: &AppState) -> Result<Tx, AppError> {
        Ok(begin_scoped(&state.db, Some(self.user_id), Some(self.tenant_id)).await?)
    }
}

impl FromRequestParts<AppState> for TenantCtx {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        let Path(params) = Path::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::NotFound)?;
        let slug = params.get("slug").ok_or(AppError::NotFound)?;

        // 只設使用者、不設租戶:RLS 讓我們只看得到自己所屬的店家
        let mut tx = begin_scoped(&state.db, Some(user.id), None).await?;
        let row: Option<(Uuid, Role)> = sqlx::query_as(
            "SELECT t.id, m.role FROM tenants t
             JOIN memberships m ON m.tenant_id = t.id
             WHERE t.slug = $1 AND m.user_id = $2 AND m.active AND t.status = 'active'",
        )
        .bind(slug)
        .bind(user.id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.rollback().await?;

        let (tenant_id, role) = row.ok_or(AppError::NotFound)?;
        Ok(TenantCtx {
            tenant_id,
            user_id: user.id,
            role,
        })
    }
}
