//! 請求 ID、結構化日誌的 span、Prometheus 指標。
//!
//! 重要:日誌與指標的路徑一律用「路由樣板」(`/public/bookings/{token}`),
//! 不用實際網址。顧客的預約管理 token 就在網址裡,寫進日誌等於洩漏取消 / 改期的權限。

use std::time::Instant;

use axum::{
    extract::{MatchedPath, Request, State},
    http::{HeaderMap, HeaderValue, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use metrics::{counter, histogram};
use tracing::Span;
use uuid::Uuid;

use crate::{db::begin_worker, error::AppError, routes::AppState};

const REQUEST_ID_HEADER: &str = "x-request-id";

/// 只接受簡單的字元與長度:請求 ID 會進日誌,不能讓用戶端塞入換行等內容偽造日誌行
fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// 沿用用戶端(或上游代理)帶來的合法 `X-Request-Id`,否則產生新的;並回寫到回應標頭
pub async fn request_id(mut req: Request, next: Next) -> Response {
    let id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| valid_request_id(v))
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    // 上面已驗證只含安全字元,轉成標頭值不會失敗
    let value = HeaderValue::from_str(&id).expect("已驗證的請求 ID");
    req.headers_mut().insert(REQUEST_ID_HEADER, value.clone());
    let mut res = next.run(req).await;
    res.headers_mut().insert(REQUEST_ID_HEADER, value);
    res
}

fn route_template(req: &Request) -> String {
    req.extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".to_owned())
}

/// TraceLayer 的 span:方法、路由樣板、請求 ID,不含實際 URI
pub fn make_span(req: &Request) -> Span {
    let request_id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-");
    tracing::info_span!(
        "request",
        method = %req.method(),
        path = %route_template(req),
        request_id = %request_id,
    )
}

/// 請求計數與延遲;label 只用方法、路由樣板、狀態碼,基數有限
pub async fn track_metrics(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = route_template(&req);
    let res = next.run(req).await;
    let status = res.status().as_u16().to_string();
    counter!("http_requests_total", "method" => method.clone(), "path" => path.clone(), "status" => status)
        .increment(1);
    histogram!("http_request_duration_seconds", "method" => method, "path" => path)
        .record(start.elapsed().as_secs_f64());
    res
}

/// 固定時間比對,避免從回應時間猜出 token
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// `GET /metrics`:未設定 `METRICS_TOKEN` 時整個端點不存在(404);設定後需帶 Bearer token。
pub async fn metrics_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (Some(handle), Some(expected)) = (&state.metrics, &state.metrics_token) else {
        return Err(AppError::NotFound);
    };
    let authorized =
        bearer(&headers).is_some_and(|t| constant_time_eq(t.as_bytes(), expected.as_bytes()));
    if !authorized {
        return Err(AppError::Unauthorized);
    }

    let mut body = handle.render();
    body.push_str(&outbox_gauges(&state).await);
    Ok((
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response())
}

/// 寄信佇列的即時狀態,每次被抓取時才查。查詢失敗不讓整個端點失敗,而是回報 `app_db_up 0`。
async fn outbox_gauges(state: &AppState) -> String {
    let result: Result<(i64, i64, f64), sqlx::Error> = async {
        let mut tx = begin_worker(&state.db).await?;
        let row = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE status = 'pending'),
                    count(*) FILTER (WHERE status = 'failed'),
                    COALESCE(extract(epoch FROM now() - min(run_at)
                             FILTER (WHERE status = 'pending' AND run_at <= now())), 0)::float8
             FROM email_outbox",
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.rollback().await?;
        Ok(row)
    }
    .await;

    match result {
        Ok((pending, failed, oldest)) => format!(
            "# TYPE app_db_up gauge\napp_db_up 1\n\
             # HELP email_outbox_pending 等待寄出的信件數\n# TYPE email_outbox_pending gauge\nemail_outbox_pending {pending}\n\
             # HELP email_outbox_failed 最終寄送失敗的信件數(保留期內)\n# TYPE email_outbox_failed gauge\nemail_outbox_failed {failed}\n\
             # HELP email_outbox_oldest_due_age_seconds 已到期但尚未寄出的最舊信件等了多久\n# TYPE email_outbox_oldest_due_age_seconds gauge\nemail_outbox_oldest_due_age_seconds {oldest}\n"
        ),
        Err(err) => {
            tracing::error!(error = %err, "讀取 outbox 指標失敗");
            "# TYPE app_db_up gauge\napp_db_up 0\n".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_cannot_forge_log_lines() {
        assert!(valid_request_id("abc-123_DEF"));
        assert!(valid_request_id(&"a".repeat(64)));
        assert!(!valid_request_id(""));
        assert!(!valid_request_id(&"a".repeat(65)));
        assert!(!valid_request_id("line1\nINFO fake log"));
        assert!(!valid_request_id("a b"));
        assert!(!valid_request_id("a\"b"));
    }

    #[test]
    fn constant_time_eq_behaves_like_eq() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secret2"));
        assert!(!constant_time_eq(b"", b"x"));
    }
}
