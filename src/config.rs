use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    pub jwt_secret: String,
    pub jwt_ttl_secs: u64,
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
        })
    }
}
