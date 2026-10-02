use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    pub jwt_secret: String,
    pub jwt_ttl_secs: u64,
    /// 公開端點每個來源每分鐘的請求上限
    pub public_rate_limit_per_min: u32,
    /// 是否信任反向代理傳來的 X-Forwarded-For(只有確實部署在可信任的代理後面才開)
    pub trust_proxy: bool,
    /// 例如 smtps://user:pass@smtp.example.com:465;未設定則只記錄不寄(開發用)
    pub smtp_url: Option<String>,
    pub mail_from: String,
    /// 信中連結的網址前綴(前端網址)
    pub public_base_url: String,
    pub worker_enabled: bool,
    pub worker_poll_secs: u64,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let jwt_secret = std::env::var("JWT_SECRET").context("缺少環境變數 JWT_SECRET")?;
        if jwt_secret.len() < 32 {
            bail!("JWT_SECRET 至少需要 32 個字元");
        }
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
            trust_proxy: std::env::var("TRUST_PROXY").is_ok_and(|v| v == "true" || v == "1"),
            smtp_url: std::env::var("SMTP_URL").ok().filter(|v| !v.is_empty()),
            mail_from: std::env::var("MAIL_FROM")
                .unwrap_or_else(|_| "預約系統 <noreply@localhost>".into()),
            public_base_url: std::env::var("PUBLIC_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:3001".into()),
            worker_enabled: std::env::var("WORKER_ENABLED")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true),
            worker_poll_secs: match std::env::var("WORKER_POLL_SECS") {
                Ok(v) => v.parse().context("WORKER_POLL_SECS 必須是整數")?,
                Err(_) => 5,
            },
        })
    }
}
