use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;

use super::AppState;
use crate::{
    audit::{self, Actor},
    billing::{self, Parsed, Stripe},
    error::AppError,
    tenancy::{Role, TenantCtx},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/t/{slug}/billing", get(overview))
        .route("/t/{slug}/billing/checkout", post(checkout))
        .route("/t/{slug}/billing/portal", post(portal))
        // 給 Stripe 呼叫:不需登入、不做 IP 限流(Stripe 的來源很多),以簽章驗證身分
        .route("/webhooks/stripe", post(webhook))
}

fn stripe(state: &AppState) -> Result<&Stripe, AppError> {
    state
        .stripe
        .as_deref()
        .ok_or_else(|| AppError::NotEnabled("尚未啟用線上付款".into()))
}

/// 付款相關只有店主能操作
fn require_owner(ctx: &TenantCtx) -> Result<(), AppError> {
    if ctx.role == Role::Owner {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

#[derive(Debug, FromRow)]
struct BillingRow {
    #[allow(dead_code)]
    stripe_customer_id: String,
    subscription_id: Option<String>,
    status: Option<String>,
    current_period_end: Option<DateTime<Utc>>,
    cancel_at_period_end: bool,
}

/// 有進行中的訂閱(付費中或寬限中)。已取消 / 未完成的可以重新訂閱
fn is_live(status: Option<&str>) -> bool {
    matches!(status, Some("active" | "trialing" | "past_due"))
}

async fn load(tx: &mut crate::db::Tx) -> Result<Option<BillingRow>, AppError> {
    Ok(
        sqlx::query_as::<_, BillingRow>("SELECT * FROM billing_state()")
            .fetch_optional(&mut **tx)
            .await?,
    )
}

#[derive(Debug, Serialize)]
struct Subscription {
    status: String,
    current_period_end: Option<DateTime<Utc>>,
    cancel_at_period_end: bool,
}

#[derive(Debug, Serialize)]
struct Overview {
    enabled: bool,
    /// 可以訂閱的方案 id
    purchasable: Vec<String>,
    subscription: Option<Subscription>,
    /// 有進行中的訂閱 → 只能走「管理訂閱」(變更 / 取消),不能再開一個
    can_checkout: bool,
    can_manage: bool,
}

async fn overview(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path(_slug): Path<String>,
) -> Result<Json<Overview>, AppError> {
    require_owner(&ctx)?;
    let Some(stripe) = state.stripe.as_deref() else {
        return Ok(Json(Overview {
            enabled: false,
            purchasable: vec![],
            subscription: None,
            can_checkout: false,
            can_manage: false,
        }));
    };
    let mut tx = ctx.begin(&state).await?;
    let row = load(&mut tx).await?;
    tx.rollback().await?;
    let live = row.as_ref().is_some_and(|r| is_live(r.status.as_deref()));
    Ok(Json(Overview {
        enabled: true,
        purchasable: stripe
            .purchasable_plans()
            .into_iter()
            .map(String::from)
            .collect(),
        can_checkout: !live,
        can_manage: row.is_some(),
        subscription: row.and_then(|r| {
            r.subscription_id?;
            Some(Subscription {
                status: r.status?,
                current_period_end: r.current_period_end,
                cancel_at_period_end: r.cancel_at_period_end,
            })
        }),
    }))
}

#[derive(Debug, Deserialize)]
struct CheckoutBody {
    plan: String,
}

#[derive(Debug, Serialize)]
struct Redirect {
    url: String,
}

async fn checkout(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path(slug): Path<String>,
    Json(req): Json<CheckoutBody>,
) -> Result<Json<Redirect>, AppError> {
    require_owner(&ctx)?;
    let stripe = stripe(&state)?;
    let price = stripe
        .price_for(&req.plan)
        .ok_or_else(|| AppError::BadRequest("這個方案目前無法線上訂閱".into()))?;

    let mut tx = ctx.begin(&state).await?;
    let existing = load(&mut tx).await?;
    if existing
        .as_ref()
        .is_some_and(|r| is_live(r.status.as_deref()))
    {
        return Err(AppError::Conflict(
            "已經有進行中的訂閱,請使用「管理訂閱」變更或取消".into(),
        ));
    }
    let shop_name: String = sqlx::query_scalar("SELECT name FROM tenants WHERE id = $1")
        .bind(ctx.tenant_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.rollback().await?; // 呼叫 Stripe 的期間不佔著資料庫連線

    let customer = match existing {
        Some(r) => r.stripe_customer_id,
        None => stripe.create_customer(ctx.tenant_id, &shop_name).await?,
    };
    let mut tx = ctx.begin(&state).await?;
    // 同時兩個請求:只會綁定一個客戶,另一個拿到已綁定的
    let customer: String = sqlx::query_scalar("SELECT billing_attach_customer($1)")
        .bind(&customer)
        .fetch_one(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        ctx.tenant_id,
        Actor::User(ctx.user_id),
        "billing.checkout_started",
        "tenant",
        Some(ctx.tenant_id),
        json!({ "plan": req.plan }),
    )
    .await?;
    tx.commit().await?;

    let base = state.public_base_url.trim_end_matches('/');
    let url = stripe
        .checkout_url(
            &customer,
            price,
            ctx.tenant_id,
            &format!("{base}/admin/{slug}/plan?checkout=success"),
            &format!("{base}/admin/{slug}/plan?checkout=cancel"),
        )
        .await?;
    Ok(Json(Redirect { url }))
}

async fn portal(
    State(state): State<AppState>,
    ctx: TenantCtx,
    Path(slug): Path<String>,
) -> Result<Json<Redirect>, AppError> {
    require_owner(&ctx)?;
    let stripe = stripe(&state)?;
    let mut tx = ctx.begin(&state).await?;
    let row = load(&mut tx).await?;
    tx.rollback().await?;
    let row = row.ok_or_else(|| AppError::Conflict("尚未訂閱任何方案".into()))?;
    let base = state.public_base_url.trim_end_matches('/');
    let url = stripe
        .portal_url(
            &row.stripe_customer_id,
            &format!("{base}/admin/{slug}/plan"),
        )
        .await?;
    Ok(Json(Redirect { url }))
}

/// Stripe 呼叫的 webhook。
/// 簽章驗證用**原始 body**(不能先解析再序列化);驗證失敗回 400;
/// 與方案無關的事件、重複的事件、過期(亂序)的事件都回 200,Stripe 才不會一直重送。
async fn webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    let stripe = stripe(&state)?;
    let signature = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("缺少簽章".into()))?;
    billing::verify_signature(
        stripe.webhook_secret(),
        signature,
        &body,
        Utc::now().timestamp(),
    )
    .map_err(|e| {
        tracing::warn!(error = ?e, "Stripe webhook 簽章驗證失敗");
        AppError::BadRequest("簽章驗證失敗".into())
    })?;

    let parsed = billing::parse_event(&body, |price| stripe.plan_for(price).map(String::from))
        .map_err(|e| {
            tracing::error!(error = %e, "無法解析 Stripe 事件");
            AppError::BadRequest("事件格式不正確".into())
        })?;
    let Parsed::Subscription(ev) = parsed else {
        return Ok(Json(json!({ "received": true, "result": "ignored" })));
    };

    // 付費中卻對不到方案 = 設定錯誤(price id 沒設定)。回 500 讓 Stripe 稍後重送,而不是默默吞掉
    // (函式在這之前不會記錄事件,所以修好設定後重送會正常處理)
    let paid = matches!(ev.status.as_str(), "active" | "trialing" | "past_due");
    if paid && ev.plan.is_none() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "訂閱 {} 的價格不在 STRIPE_PRICE_* 設定內",
            ev.subscription
        )));
    }

    let result: String = sqlx::query_scalar(
        "SELECT billing_apply_event($1, $2, $3, $4, $5, $6, $7, to_timestamp($8), $9)",
    )
    .bind(&ev.event_id)
    .bind(&ev.kind)
    .bind(ev.created)
    .bind(&ev.customer)
    .bind(&ev.subscription)
    .bind(&ev.status)
    .bind(&ev.plan)
    .bind(ev.period_end.map(|t| t as f64))
    .bind(ev.cancel_at_period_end)
    .fetch_one(&state.db)
    .await?;
    tracing::info!(event = %ev.event_id, kind = %ev.kind, %result, "Stripe 事件已處理");
    Ok(Json(json!({ "received": true, "result": result })))
}
