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
    pub worker_enabled: bool,
    pub worker_poll_secs: u64,
    /// 設定後才會開放 `GET /metrics`(需帶 Bearer token);未設定則端點不存在
    pub metrics_token: Option<String>,
    /// `APP_ENV=production`:啟動時強制檢查資料庫帳號夠不夠受限,並預設不自動套用 migration
    pub production: bool,
    pub auto_migrate: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let jwt_secret = std::env::var("JWT_SECRET").context("缺少環境變數 JWT_SECRET")?;
        if jwt_secret.len() < 32 {
            bail!("JWT_SECRET 至少需要 32 個字元");
        }
        let metrics_token = parse_metrics_token(std::env::var("METRICS_TOKEN").ok())?;
        let production = std::env::var("APP_ENV").is_ok_and(|v| v == "production");
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
            public_base_url: std::env::var("PUBLIC_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:3001".into()),
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
            worker_poll_secs: match std::env::var("WORKER_POLL_SECS") {
                Ok(v) => v.parse().context("WORKER_POLL_SECS 必須是整數")?,
                Err(_) => 5,
            },
        })
    }
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
