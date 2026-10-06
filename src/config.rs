use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    pub jwt_secret: String,
    pub jwt_ttl_secs: u64,
    /// 公開端點每個來源每分鐘的請求上限
    pub public_rate_limit_per_min: u32,
    /// 註冊 / 登入每個來源每分鐘的請求上限
    pub auth_rate_limit_per_min: u32,
    /// 是否信任反向代理傳來的 X-Forwarded-For(只有確實部署在可信任的代理後面才開)
    pub trust_proxy: bool,
    /// 例如 smtps://user:pass@smtp.example.com:465;未設定則只記錄不寄(開發用)
    pub smtp_url: Option<String>,
    pub mail_from: String,
    /// 信中連結的網址前綴(前端網址)
    pub public_base_url: String,
    /// 每位使用者最多能「擁有」幾家店(防止灌店家、占用代稱)
    pub max_shops_per_user: u32,
    /// 稽核日誌保留天數(超過的由背景任務清除)。None = 永久保留
    pub audit_retention_days: Option<u32>,
    /// 沒驗證 Email 的使用者不能建立店家。只有開發 / 測試能關掉,production 不允許
    pub require_verified_email: bool,
    /// 同一個收件者每小時最多收幾封信(跨店家;防止拿別人的 Email 當收件者騷擾)
    pub max_mails_per_recipient_per_hour: u32,
    /// 以 cookie 登入時,允許送出「會改資料」請求的網頁來源(CSRF 防護,見 session.rs)
    pub allowed_origins: Vec<String>,
    pub worker_enabled: bool,
    pub worker_poll_secs: u64,
    /// 設定後才會開放 `GET /metrics`(需帶 Bearer token);未設定則端點不存在
    pub metrics_token: Option<String>,
    /// `APP_ENV=production`:啟動時強制檢查資料庫帳號夠不夠受限,並預設不自動套用 migration
    pub production: bool,
    pub auto_migrate: bool,
    pub stripe: Option<StripeConfig>,
}

/// 計費設定。`STRIPE_SECRET_KEY` 與 `STRIPE_WEBHOOK_SECRET` 都有才啟用;沒設定時方案只能由營運人員調整
#[derive(Debug, Clone)]
pub struct StripeConfig {
    pub secret_key: String,
    pub webhook_secret: String,
    /// 預設 https://api.stripe.com;測試時指向 stripe-mock 或假伺服器
    pub api_base: String,
    /// 方案 id(pro / business)→ Stripe price id
    pub prices: Vec<(String, String)>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let jwt_secret = std::env::var("JWT_SECRET").context("缺少環境變數 JWT_SECRET")?;
        if jwt_secret.len() < 32 {
            bail!("JWT_SECRET 至少需要 32 個字元");
        }
        let metrics_token = parse_metrics_token(std::env::var("METRICS_TOKEN").ok())?;
        let production = std::env::var("APP_ENV").is_ok_and(|v| v == "production");
        let public_base_url =
            std::env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "http://localhost:3001".into());
        let allowed_origins = crate::session::allowed_origins(
            &public_base_url,
            std::env::var("ALLOWED_ORIGINS").ok().as_deref(),
            production,
        );
        Ok(Self {
            database_url: std::env::var("DATABASE_URL").context("缺少環境變數 DATABASE_URL")?,
            bind_addr: std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:3001".into()),
            jwt_secret,
            jwt_ttl_secs: match std::env::var("JWT_TTL_SECS") {
                Ok(v) => v.parse().context("JWT_TTL_SECS 必須是整數")?,
                Err(_) => 3600,
            },
            public_rate_limit_per_min: match std::env::var("PUBLIC_RATE_LIMIT_PER_MIN") {
                Ok(v) => v.parse().context("PUBLIC_RATE_LIMIT_PER_MIN 必須是整數")?,
                Err(_) => 60,
            },
            auth_rate_limit_per_min: match std::env::var("AUTH_RATE_LIMIT_PER_MIN") {
                Ok(v) => v.parse().context("AUTH_RATE_LIMIT_PER_MIN 必須是整數")?,
                Err(_) => 10,
            },
            trust_proxy: std::env::var("TRUST_PROXY").is_ok_and(|v| v == "true" || v == "1"),
            smtp_url: std::env::var("SMTP_URL").ok().filter(|v| !v.is_empty()),
            mail_from: std::env::var("MAIL_FROM")
                .unwrap_or_else(|_| "預約系統 <noreply@localhost>".into()),
            public_base_url,
            max_shops_per_user: parse_max_shops(std::env::var("MAX_SHOPS_PER_USER").ok())?,
            audit_retention_days: parse_audit_retention(
                std::env::var("AUDIT_RETENTION_DAYS").ok().as_deref(),
            )?,
            require_verified_email: parse_require_verified(
                std::env::var("REQUIRE_VERIFIED_EMAIL").ok(),
                production,
            )?,
            max_mails_per_recipient_per_hour: parse_positive(
                "MAX_MAILS_PER_RECIPIENT_PER_HOUR",
                std::env::var("MAX_MAILS_PER_RECIPIENT_PER_HOUR").ok(),
                6,
            )?,
            allowed_origins,
            worker_enabled: std::env::var("WORKER_ENABLED")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true),
            metrics_token,
            production,
            // 開發預設自動套用;正式環境預設不套用(執行階段帳號本來就不該有建表權限),要明確設 true 才會
            auto_migrate: match std::env::var("AUTO_MIGRATE").ok().as_deref() {
                Some("true" | "1") => true,
                Some("false" | "0") => false,
                _ => !production,
            },
            stripe: parse_stripe(|k| std::env::var(k).ok())?,
            worker_poll_secs: match std::env::var("WORKER_POLL_SECS") {
                Ok(v) => v.parse().context("WORKER_POLL_SECS 必須是整數")?,
                Err(_) => 5,
            },
        })
    }
}

/// `MAX_SHOPS_PER_USER`:沒設定 = 5;0 會讓所有人都建立不了店家,視為設定錯誤
pub fn parse_max_shops(value: Option<String>) -> Result<u32> {
    parse_positive("MAX_SHOPS_PER_USER", value, 5)
}

/// 正整數設定:沒設定 / 空字串用預設值;不是整數或是 0 都視為設定錯誤(0 會讓功能整個不能用)
pub fn parse_positive(name: &str, value: Option<String>, default: u32) -> Result<u32> {
    let Some(v) = value.filter(|v| !v.is_empty()) else {
        return Ok(default);
    };
    let n: u32 = v
        .trim()
        .parse()
        .with_context(|| format!("{name} 必須是整數"))?;
    if n == 0 {
        bail!("{name} 至少要是 1(0 會讓這個功能整個不能用)");
    }
    Ok(n)
}

/// 環境變數的讀取方式由呼叫端傳入,測試才不必修改行程全域的環境變數(會干擾平行執行的其他測試)
pub fn parse_stripe(env: impl Fn(&str) -> Option<String>) -> Result<Option<StripeConfig>> {
    let get = |k: &str| env(k).filter(|v| !v.is_empty());
    let (key, hook) = (get("STRIPE_SECRET_KEY"), get("STRIPE_WEBHOOK_SECRET"));
    let (Some(secret_key), Some(webhook_secret)) = (key.clone(), hook.clone()) else {
        if key.is_some() || hook.is_some() {
            bail!(
                "STRIPE_SECRET_KEY 與 STRIPE_WEBHOOK_SECRET 必須同時設定(只設一個會讓 webhook 收不到或無法驗證)"
            );
        }
        return Ok(None);
    };
    let prices: Vec<(String, String)> = [
        ("pro", "STRIPE_PRICE_PRO"),
        ("business", "STRIPE_PRICE_BUSINESS"),
    ]
    .into_iter()
    .filter_map(|(plan, var)| get(var).map(|p| (plan.to_string(), p)))
    .collect();
    if prices.is_empty() {
        bail!("啟用 Stripe 時至少要設定 STRIPE_PRICE_PRO 或 STRIPE_PRICE_BUSINESS");
    }
    Ok(Some(StripeConfig {
        secret_key,
        webhook_secret,
        api_base: get("STRIPE_API_BASE").unwrap_or_else(|| "https://api.stripe.com".into()),
        prices,
    }))
}

/// 空字串視為未設定(端點關閉);太短的 token 拒絕啟動,而不是默默開一個容易被猜中的端點
fn parse_metrics_token(raw: Option<String>) -> Result<Option<String>> {
    let token = raw.filter(|v| !v.is_empty());
    if token.as_ref().is_some_and(|t| t.len() < 16) {
        bail!("METRICS_TOKEN 至少需要 16 個字元");
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_token_rules() {
        assert_eq!(parse_metrics_token(None).unwrap(), None);
        assert_eq!(
            parse_metrics_token(Some(String::new())).unwrap(),
            None,
            "空字串 = 關閉"
        );
        assert!(parse_metrics_token(Some("short".into())).is_err());
        assert!(parse_metrics_token(Some("x".repeat(15))).is_err());
        assert_eq!(
            parse_metrics_token(Some("x".repeat(16))).unwrap(),
            Some("x".repeat(16))
        );
    }
}

#[cfg(test)]
mod shop_limit_tests {
    use super::parse_max_shops;

    #[test]
    fn max_shops_defaults_and_validates() {
        assert_eq!(parse_max_shops(None).unwrap(), 5);
        assert_eq!(parse_max_shops(Some(String::new())).unwrap(), 5);
        assert_eq!(parse_max_shops(Some("1".into())).unwrap(), 1);
        assert_eq!(parse_max_shops(Some(" 20 ".into())).unwrap(), 20);
        for bad in ["0", "-1", "many", "1.5"] {
            assert!(parse_max_shops(Some(bad.into())).is_err(), "{bad}");
        }
    }
}

/// 預設要求驗證;只有明確寫 false / 0 才關,而且 production 不允許關
fn parse_require_verified(raw: Option<String>, production: bool) -> Result<bool> {
    let off = raw.is_some_and(|v| v == "false" || v == "0");
    if off && production {
        anyhow::bail!("production 不允許關閉 REQUIRE_VERIFIED_EMAIL");
    }
    Ok(!off)
}

#[cfg(test)]
mod verified_tests {
    use super::parse_require_verified;

    #[test]
    fn verification_is_required_unless_explicitly_disabled_outside_production() {
        assert!(parse_require_verified(None, true).unwrap());
        assert!(parse_require_verified(Some("true".into()), false).unwrap());
        assert!(parse_require_verified(Some("".into()), false).unwrap());
        assert!(!parse_require_verified(Some("false".into()), false).unwrap());
        assert!(!parse_require_verified(Some("0".into()), false).unwrap());
        assert!(parse_require_verified(Some("false".into()), true).is_err());
    }
}

/// 稽核日誌保留天數:預設 730;`0` = 永久保留;其他值不得少於 90(太短等於沒有稽核)
pub fn parse_audit_retention(raw: Option<&str>) -> Result<Option<u32>> {
    match raw.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(Some(730)),
        Some(v) => match v.parse::<u32>() {
            Ok(0) => Ok(None),
            Ok(n) if n >= 90 => Ok(Some(n)),
            Ok(_) => bail!("AUDIT_RETENTION_DAYS 不得少於 90(0 = 永久保留)"),
            Err(_) => bail!("AUDIT_RETENTION_DAYS 必須是整數"),
        },
    }
}

#[cfg(test)]
mod audit_retention_tests {
    use super::parse_audit_retention;

    #[test]
    fn defaults_to_two_years_and_rejects_values_that_would_gut_the_audit_trail() {
        assert_eq!(parse_audit_retention(None).unwrap(), Some(730));
        assert_eq!(parse_audit_retention(Some("")).unwrap(), Some(730));
        assert_eq!(parse_audit_retention(Some("365")).unwrap(), Some(365));
        assert_eq!(parse_audit_retention(Some("90")).unwrap(), Some(90));
        assert_eq!(parse_audit_retention(Some("0")).unwrap(), None);
        assert!(parse_audit_retention(Some("89")).is_err());
        assert!(parse_audit_retention(Some("1")).is_err());
        assert!(parse_audit_retention(Some("-5")).is_err());
        assert!(parse_audit_retention(Some("two years")).is_err());
    }
}
