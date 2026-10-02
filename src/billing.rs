//! 計費(Stripe)。
//!
//! 只用到三個 API:建立客戶、建立結帳 Session(Checkout)、建立客戶入口 Session(Customer Portal)。
//! 方案何時生效完全由 webhook 決定(不相信瀏覽器導回來的網址參數)。

use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha2::Sha256;
use uuid::Uuid;

use crate::{config::StripeConfig, error::AppError};

/// webhook 簽章的時間容忍度(秒)。太舊的簽章拒絕,避免被重放
pub const SIGNATURE_TOLERANCE_SECS: i64 = 300;

pub struct Stripe {
    http: reqwest::Client,
    cfg: StripeConfig,
}

impl Stripe {
    pub fn new(cfg: StripeConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("HTTP client");
        Self { http, cfg }
    }

    pub fn webhook_secret(&self) -> &str {
        &self.cfg.webhook_secret
    }

    /// 方案可不可以買(有沒有設定對應的 price)
    pub fn price_for(&self, plan: &str) -> Option<&str> {
        self.cfg
            .prices
            .iter()
            .find(|(p, _)| p == plan)
            .map(|(_, price)| price.as_str())
    }

    pub fn plan_for(&self, price: &str) -> Option<&str> {
        self.cfg
            .prices
            .iter()
            .find(|(_, p)| p == price)
            .map(|(plan, _)| plan.as_str())
    }

    pub fn purchasable_plans(&self) -> Vec<&str> {
        self.cfg.prices.iter().map(|(p, _)| p.as_str()).collect()
    }

    async fn post(
        &self,
        path: &str,
        form: &[(&str, String)],
        idempotency_key: Option<&str>,
    ) -> Result<Value, AppError> {
        let mut req = self
            .http
            .post(format!("{}{path}", self.cfg.api_base.trim_end_matches('/')))
            .bearer_auth(&self.cfg.secret_key)
            .form(form);
        if let Some(key) = idempotency_key {
            req = req.header("Idempotency-Key", key);
        }
        let res = req.send().await.map_err(|e| {
            tracing::error!(error = %e, path, "呼叫 Stripe 失敗");
            upstream()
        })?;
        let status = res.status();
        let body: Value = res.json().await.map_err(|e| {
            tracing::error!(error = %e, path, %status, "Stripe 回應不是 JSON");
            upstream()
        })?;
        if !status.is_success() {
            // Stripe 的錯誤訊息只進日誌,不回給使用者(可能含內部細節)
            tracing::error!(path, %status, error = %body["error"]["message"], "Stripe 回傳錯誤");
            return Err(upstream());
        }
        Ok(body)
    }

    /// 同一店家用同一個 idempotency key:重試或同時兩次請求只會建立一個客戶
    pub async fn create_customer(&self, tenant_id: Uuid, name: &str) -> Result<String, AppError> {
        let body = self
            .post(
                "/v1/customers",
                &[
                    ("name", name.to_string()),
                    ("metadata[tenant_id]", tenant_id.to_string()),
                ],
                Some(&format!("tenant-customer-{tenant_id}")),
            )
            .await?;
        string_field(&body, "id")
    }

    pub async fn checkout_url(
        &self,
        customer: &str,
        price: &str,
        tenant_id: Uuid,
        success_url: &str,
        cancel_url: &str,
    ) -> Result<String, AppError> {
        let body = self
            .post(
                "/v1/checkout/sessions",
                &[
                    ("mode", "subscription".into()),
                    ("customer", customer.into()),
                    ("line_items[0][price]", price.into()),
                    ("line_items[0][quantity]", "1".into()),
                    ("success_url", success_url.into()),
                    ("cancel_url", cancel_url.into()),
                    ("client_reference_id", tenant_id.to_string()),
                    (
                        "subscription_data[metadata][tenant_id]",
                        tenant_id.to_string(),
                    ),
                ],
                None,
            )
            .await?;
        string_field(&body, "url")
    }

    pub async fn portal_url(&self, customer: &str, return_url: &str) -> Result<String, AppError> {
        let body = self
            .post(
                "/v1/billing_portal/sessions",
                &[
                    ("customer", customer.into()),
                    ("return_url", return_url.into()),
                ],
                None,
            )
            .await?;
        string_field(&body, "url")
    }
}

fn upstream() -> AppError {
    AppError::Upstream("付款服務暫時無法使用,請稍後再試".into())
}

fn string_field(body: &Value, key: &str) -> Result<String, AppError> {
    body[key].as_str().map(str::to_string).ok_or_else(|| {
        tracing::error!(key, "Stripe 回應缺少欄位");
        upstream()
    })
}

// ---------- webhook 簽章 ----------

#[derive(Debug, PartialEq, Eq)]
pub enum SignatureError {
    Malformed,
    Expired,
    Mismatch,
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// 驗證 `Stripe-Signature: t=<秒>,v1=<hex>[,v1=<hex>…]`。
/// 簽章內容是 `"{t}.{原始 body}"` 的 HMAC-SHA256;比對用常數時間,且時間戳在容忍範圍內。
pub fn verify_signature(
    secret: &str,
    header: &str,
    payload: &[u8],
    now_secs: i64,
) -> Result<(), SignatureError> {
    let mut timestamp: Option<i64> = None;
    let mut candidates: Vec<Vec<u8>> = Vec::new();
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => timestamp = v.parse().ok(),
            Some(("v1", v)) => candidates.extend(unhex(v)),
            _ => {}
        }
    }
    let t = timestamp.ok_or(SignatureError::Malformed)?;
    if candidates.is_empty() {
        return Err(SignatureError::Malformed);
    }
    if (now_secs - t).abs() > SIGNATURE_TOLERANCE_SECS {
        return Err(SignatureError::Expired);
    }
    let matches = candidates.iter().any(|sig| {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC 接受任何長度的金鑰");
        mac.update(t.to_string().as_bytes());
        mac.update(b".");
        mac.update(payload);
        mac.verify_slice(sig).is_ok()
    });
    if matches {
        Ok(())
    } else {
        Err(SignatureError::Mismatch)
    }
}

/// 產生簽章(測試與本機驗證用)
pub fn sign(secret: &str, timestamp: i64, payload: &[u8]) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC 接受任何長度的金鑰");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(payload);
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("t={timestamp},v1={hex}")
}

// ---------- 事件 ----------

/// 我們關心的訂閱事件(已換算成方案)
#[derive(Debug, PartialEq)]
pub struct SubscriptionEvent {
    pub event_id: String,
    pub kind: String,
    pub created: i64,
    pub customer: String,
    pub subscription: String,
    pub status: String,
    /// 這個訂閱對應的方案;價格不在設定內時為 None
    pub plan: Option<String>,
    pub period_end: Option<i64>,
    pub cancel_at_period_end: bool,
}

pub enum Parsed {
    Subscription(SubscriptionEvent),
    /// 與方案無關的事件:收下並回 200,Stripe 才不會一直重送
    Ignored,
}

const SUBSCRIPTION_EVENTS: &[&str] = &[
    "customer.subscription.created",
    "customer.subscription.updated",
    "customer.subscription.deleted",
];

pub fn parse_event(
    raw: &[u8],
    plan_for: impl Fn(&str) -> Option<String>,
) -> Result<Parsed, String> {
    let event: Value =
        serde_json::from_slice(raw).map_err(|e| format!("事件不是有效的 JSON: {e}"))?;
    let kind = event["type"].as_str().ok_or("事件缺少 type")?;
    if !SUBSCRIPTION_EVENTS.contains(&kind) {
        return Ok(Parsed::Ignored);
    }
    let text = |v: &Value, what: &str| {
        v.as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("訂閱事件缺少 {what}"))
    };
    let obj = &event["data"]["object"];
    let item = &obj["items"]["data"][0];
    let price = item["price"]["id"].as_str();
    Ok(Parsed::Subscription(SubscriptionEvent {
        event_id: text(&event["id"], "id")?,
        kind: kind.to_string(),
        created: event["created"].as_i64().ok_or("事件缺少 created")?,
        customer: text(&obj["customer"], "customer")?,
        subscription: text(&obj["id"], "subscription id")?,
        status: text(&obj["status"], "status")?,
        plan: price.and_then(&plan_for),
        // 新版 API 把週期結束時間放在訂閱項目上,舊版放在訂閱本身
        period_end: obj["current_period_end"]
            .as_i64()
            .or_else(|| item["current_period_end"].as_i64()),
        cancel_at_period_end: obj["cancel_at_period_end"].as_bool().unwrap_or(false),
    }))
}
