//! 寄信:信件內容範本與寄送後端。
//!
//! 後端有三種:`Log`(開發用,只記錄不寄)、`Smtp`(正式)、`Memory`(測試用,可模擬失敗)。

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
};

use crate::config::Config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email {
    pub to: String,
    pub subject: String,
    pub body: String,
}

#[derive(Clone, Default)]
pub struct MemoryMailer {
    sent: Arc<Mutex<Vec<Email>>>,
    fail_remaining: Arc<AtomicUsize>,
}

impl MemoryMailer {
    pub fn new() -> Self {
        Self::default()
    }

    /// 前 `n` 次寄送都會失敗,用來測試重試與退避
    pub fn failing(n: usize) -> Self {
        let mailer = Self::default();
        mailer.fail_remaining.store(n, Ordering::SeqCst);
        mailer
    }

    pub fn sent(&self) -> Vec<Email> {
        self.sent.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

pub enum Mailer {
    Log,
    Smtp {
        transport: Box<AsyncSmtpTransport<Tokio1Executor>>,
        from: Mailbox,
    },
    Memory(MemoryMailer),
}

impl Mailer {
    pub fn from_config(config: &Config) -> Result<Self> {
        let Some(url) = &config.smtp_url else {
            // Log 模式會把信件內容(含一次性連結)印進日誌,只能給開發用。
            // 正式環境沒有寄信設定就不該啟動,否則取得日誌的人就能取消 / 改期任何顧客的預約。
            if config.production {
                anyhow::bail!(
                    "APP_ENV=production 必須設定 SMTP_URL(未設定時信件會被印進日誌,內含一次性連結)。\
                     若暫時不需要寄信,請設定 WORKER_ENABLED=false"
                );
            }
            tracing::warn!(
                "未設定 SMTP_URL:信件(含連結)只會印在日誌,不會真的寄出。正式環境請設定 SMTP_URL"
            );
            return Ok(Mailer::Log);
        };
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
            .context("SMTP_URL 格式不正確")?
            .timeout(Some(std::time::Duration::from_secs(20)))
            .build();
        let from = config.mail_from.parse().context("MAIL_FROM 格式不正確")?;
        Ok(Mailer::Smtp {
            transport: Box::new(transport),
            from,
        })
    }

    pub async fn send(&self, email: &Email) -> Result<()> {
        match self {
            Mailer::Log => {
                // 只有開發用的 Log 模式才印出內容(含一次性連結),否則開發者走不完確認流程。
                // 正式環境一定要設定 SMTP_URL;啟動時已提醒。
                tracing::info!(to = %email.to, subject = %email.subject, "(Log 模式)模擬寄信\n{}", email.body);
                Ok(())
            }
            Mailer::Memory(m) => {
                let failed = m
                    .fail_remaining
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_ok();
                if failed {
                    return Err(anyhow!("模擬寄送失敗"));
                }
                m.sent
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(email.clone());
                Ok(())
            }
            Mailer::Smtp { transport, from } => {
                let message = Message::builder()
                    .from(from.clone())
                    .to(email.to.parse().context("收件者格式不正確")?)
                    // 主旨含店家名稱(使用者輸入),去掉換行避免標頭注入
                    .subject(email.subject.replace(['\r', '\n'], " "))
                    .header(ContentType::TEXT_PLAIN)
                    .body(email.body.clone())
                    .context("組信失敗")?;
                transport.send(message).await.context("SMTP 寄送失敗")?;
                Ok(())
            }
        }
    }
}

// ---------- 範本 ----------

pub fn format_local(at: DateTime<Utc>, tz: Tz) -> String {
    at.with_timezone(&tz)
        .format("%Y-%m-%d %H:%M (%Z)")
        .to_string()
}

fn base(url: &str) -> &str {
    url.trim_end_matches('/')
}

pub struct BookingMail<'a> {
    pub to: &'a str,
    pub shop: &'a str,
    pub service: &'a str,
    pub staff: &'a str,
    pub when: &'a str,
}

pub fn booking_link(base_url: &str, token: &str) -> String {
    format!("{}/bookings/{token}", base(base_url))
}

pub fn invitation_link(base_url: &str, token: &str) -> String {
    format!("{}/invitations/accept#token={token}", base(base_url))
}

pub fn verification(m: &BookingMail, link: &str) -> Email {
    Email {
        to: m.to.to_string(),
        subject: format!("請確認您在「{}」的預約", m.shop),
        body: format!(
            "您好,\n\n我們收到一筆預約申請:\n\n  店家:{}\n  服務:{}\n  人員:{}\n  時間:{}\n\n\
             時段在您確認之前不會保留。請開啟下列連結並按下「確認預約」:\n\n  {link}\n\n\
             連結 24 小時內有效。如果這不是您本人的操作,請忽略這封信,不會有任何預約成立。\n",
            m.shop, m.service, m.staff, m.when
        ),
    }
}

pub fn confirmed(m: &BookingMail, link: &str) -> Email {
    Email {
        to: m.to.to_string(),
        subject: format!("預約已確認:{}", m.shop),
        body: format!(
            "您好,\n\n您的預約已確認:\n\n  店家:{}\n  服務:{}\n  人員:{}\n  時間:{}\n\n\
             需要查看、改期或取消,請使用這個連結(請勿轉寄給他人):\n\n  {link}\n",
            m.shop, m.service, m.staff, m.when
        ),
    }
}

/// 提醒信不含管理連結:資料庫只存雜湊,原始 token 寄出後就不保留
pub fn reminder(m: &BookingMail) -> Email {
    Email {
        to: m.to.to_string(),
        subject: format!("預約提醒:{}", m.shop),
        body: format!(
            "您好,\n\n提醒您即將到來的預約:\n\n  店家:{}\n  服務:{}\n  人員:{}\n  時間:{}\n\n\
             如需取消或改期,請使用預約確認信中的連結。\n",
            m.shop, m.service, m.staff, m.when
        ),
    }
}

/// 員工替顧客改期後的通知。不含管理連結(資料庫只存雜湊),改期 / 取消請用確認信中的連結
pub fn rescheduled(m: &BookingMail, old_when: &str) -> Email {
    Email {
        to: m.to.to_string(),
        subject: format!("預約時間已變更:{}", m.shop),
        body: format!(
            "您好,\n\n店家已為您調整預約時間:\n\n  店家:{}\n  服務:{}\n  人員:{}\n  原時間:{}\n  新時間:{}\n\n\
             如果這個時間不方便,請使用預約確認信中的連結改期或取消。\n",
            m.shop, m.service, m.staff, old_when, m.when
        ),
    }
}

pub fn password_reset_link(base_url: &str, token: &str) -> String {
    // token 放在 # 之後:不會送到任何伺服器、不進存取紀錄與 Referer
    format!("{}/admin/reset#token={token}", base(base_url))
}

pub fn password_reset(to: &str, link: &str) -> Email {
    Email {
        to: to.to_string(),
        subject: "重設您的密碼".to_string(),
        body: format!(
            "您好,\n\n我們收到重設密碼的申請。請開啟下列連結設定新密碼:\n\n  {link}\n\n\
             連結 1 小時內有效,而且只能使用一次。\n\
             如果這不是您本人的操作,請忽略這封信,您的密碼不會改變。\n"
        ),
    }
}

pub fn invitation(to: &str, shop: &str, role: &str, link: &str) -> Email {
    Email {
        to: to.to_string(),
        subject: format!("您被邀請加入「{shop}」"),
        body: format!(
            "您好,\n\n您被邀請以「{role}」身分加入「{shop}」。\n\n\
             請先使用這個 Email 註冊或登入,再開啟下列連結接受邀請:\n\n  {link}\n\n\
             連結 7 天內有效,且只能由被邀請的 Email 使用。\n"
        ),
    }
}
