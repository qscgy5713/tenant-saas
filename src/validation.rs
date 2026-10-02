use crate::error::AppError;

/// 去除前後空白並檢查 Email 格式(粗略檢查,真正的驗證靠寄信確認)
pub fn normalize_email(raw: &str) -> Result<String, AppError> {
    let email = raw.trim();
    let valid = email.len() <= 254
        && !email.chars().any(char::is_whitespace)
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !domain.contains('@')
        });
    if valid {
        Ok(email.to_string())
    } else {
        Err(AppError::BadRequest("Email 格式不正確".into()))
    }
}
