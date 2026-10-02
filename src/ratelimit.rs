use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

/// 簡單的固定視窗限流(單一程序、記憶體內)。
/// 部署成多個實例時各自計算;若需要全域限流,之後改用 Redis。
pub struct RateLimiter {
    limit: u32,
    window: Duration,
    entries: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// 回傳 true 表示放行
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        // 避免被大量不同來源撐爆記憶體:過大時順手清掉已過期的
        if entries.len() > 10_000 {
            entries.retain(|_, (start, _)| now.duration_since(*start) < self.window);
        }
        let entry = entries.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= self.limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_limit_and_is_per_key() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check("a"));
        assert!(limiter.check("a"));
        assert!(!limiter.check("a"));
        assert!(limiter.check("b"), "不同來源各自計算");
    }

    #[test]
    fn window_resets() {
        let limiter = RateLimiter::new(1, Duration::from_millis(30));
        assert!(limiter.check("a"));
        assert!(!limiter.check("a"));
        std::thread::sleep(Duration::from_millis(50));
        assert!(limiter.check("a"));
    }
}
