//! 瀏覽器登入狀態:JWT 放在 HttpOnly cookie,頁面上的 JavaScript(包含 XSS 注入的)讀不到。
//!
//! 威脅與對策:
//! * XSS 偷 token → `HttpOnly`(JS 讀不到);正式環境再加 `Secure` 與 `__Host-` 前綴(不可被子網域覆寫)
//! * CSRF → `SameSite=Strict`,**再加** `Origin` 白名單檢查:`SameSite` 的「同站」是以可註冊網域
//!   為單位,同網域下的其他子網域不被擋,所以不能只靠它
//! * 非瀏覽器客戶端(測試、腳本)仍可用 `Authorization: Bearer`;那不是瀏覽器自動帶上的,不需要 CSRF 檢查

use axum::http::{HeaderMap, Method, header};

use crate::config::Config;

#[derive(Clone, Debug)]
pub struct CookieConfig {
    pub name: String,
    pub secure: bool,
    pub max_age_secs: u64,
    allowed_origins: Vec<String>,
}

impl CookieConfig {
    pub fn new(config: &Config) -> Self {
        Self {
            // `__Host-` 前綴要求 Secure + Path=/ + 無 Domain,瀏覽器會拒收不符合的 cookie;
            // 開發環境是 http,沒有 Secure,所以只有正式環境才用
            name: if config.production {
                "__Host-session"
            } else {
                "session"
            }
            .into(),
            secure: config.production,
            max_age_secs: config.jwt_ttl_secs,
            allowed_origins: config.allowed_origins.clone(),
        }
    }

    /// `Set-Cookie` 的值
    pub fn set(&self, token: &str) -> String {
        self.build(token, self.max_age_secs)
    }

    /// 清除 cookie(HttpOnly 的 cookie JS 刪不掉,登出一定要由伺服器做)
    pub fn clear(&self) -> String {
        self.build("", 0)
    }

    fn build(&self, value: &str, max_age: u64) -> String {
        let secure = if self.secure { "; Secure" } else { "" };
        format!(
            "{}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}",
            self.name
        )
    }

    /// 從 `Cookie` 標頭取出登入 cookie(不引入額外套件:格式是 `a=1; b=2`)
    pub fn read<'a>(&self, headers: &'a HeaderMap) -> Option<&'a str> {
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|line| line.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .find(|(name, value)| *name == self.name && !value.is_empty())
            .map(|(_, value)| value)
    }

    /// 以 cookie 認證的「會改資料」請求,`Origin` 必須在白名單內。
    /// 瀏覽器對 POST / PUT / PATCH / DELETE 一律會帶 `Origin`;沒有就當作不可信。
    pub fn origin_ok(&self, method: &Method, headers: &HeaderMap) -> bool {
        if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
            return true;
        }
        headers
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|origin| {
                self.allowed_origins
                    .iter()
                    .any(|allowed| allowed == origin.trim_end_matches('/'))
            })
    }
}

/// `https://app.example.com/admin?x=1` → `https://app.example.com`(Origin 標頭的格式)
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    if scheme.is_empty() || host.is_empty() {
        return None;
    }
    Some(format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase()
    ))
}

/// 白名單 = 公開網址的來源 + `ALLOWED_ORIGINS`(逗號分隔)+ 非正式環境的 Vite 開發伺服器
pub fn allowed_origins(
    public_base_url: &str,
    extra: Option<&str>,
    production: bool,
) -> Vec<String> {
    let mut list: Vec<String> = origin_of(public_base_url).into_iter().collect();
    list.extend(extra.unwrap_or_default().split(',').filter_map(origin_of));
    if !production {
        list.push("http://localhost:5173".into());
        list.push("http://127.0.0.1:5173".into());
    }
    list.sort();
    list.dedup();
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn cfg(production: bool) -> CookieConfig {
        CookieConfig {
            name: if production {
                "__Host-session"
            } else {
                "session"
            }
            .into(),
            secure: production,
            max_age_secs: 3600,
            allowed_origins: vec!["https://app.example.com".into()],
        }
    }

    fn headers(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(k.clone(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn set_cookie_has_the_protective_attributes() {
        let dev = cfg(false).set("abc.def.ghi");
        assert!(dev.starts_with("session=abc.def.ghi;"));
        for attr in ["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=3600"] {
            assert!(dev.contains(attr), "{dev}");
        }
        assert!(!dev.contains("Secure"), "開發環境是 http:{dev}");

        let prod = cfg(true).set("abc");
        assert!(prod.starts_with("__Host-session=abc;"));
        assert!(prod.contains("; Secure"), "{prod}");
        assert!(!prod.contains("Domain"), "__Host- 不可帶 Domain:{prod}");
    }

    #[test]
    fn clear_expires_the_cookie_with_the_same_attributes() {
        let c = cfg(true).clear();
        assert!(c.starts_with("__Host-session=;"));
        assert!(c.contains("Max-Age=0") && c.contains("HttpOnly") && c.contains("Secure"));
    }

    #[test]
    fn reads_only_the_session_cookie_by_exact_name() {
        let c = cfg(false);
        let h = headers(&[(header::COOKIE, "theme=dark; session=tok123; other=1")]);
        assert_eq!(c.read(&h), Some("tok123"));
        // 名稱要完全相符:不能把 xsession 或 session2 當成登入
        for bad in ["xsession=a", "session2=a", "SESSION=a", "session="] {
            assert_eq!(c.read(&headers(&[(header::COOKIE, bad)])), None, "{bad}");
        }
        // 多個 Cookie 標頭
        let h = headers(&[(header::COOKIE, "a=1"), (header::COOKIE, "session=zzz")]);
        assert_eq!(c.read(&h), Some("zzz"));
        assert_eq!(c.read(&HeaderMap::new()), None);
    }

    #[test]
    fn unsafe_methods_need_an_allowed_origin() {
        let c = cfg(true);
        let ok = headers(&[(header::ORIGIN, "https://app.example.com")]);
        let evil = headers(&[(header::ORIGIN, "https://evil.example.com")]);
        let sibling = headers(&[(header::ORIGIN, "https://blog.example.com")]);
        let none = HeaderMap::new();
        for m in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(c.origin_ok(&m, &ok), "{m}");
            assert!(!c.origin_ok(&m, &evil), "{m}");
            assert!(!c.origin_ok(&m, &sibling), "同網域的其他子網域也不行:{m}");
            assert!(!c.origin_ok(&m, &none), "沒有 Origin 不可信:{m}");
        }
        // 讀取不檢查
        for m in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert!(c.origin_ok(&m, &evil));
        }
        // 不能用前綴比對騙過去
        let prefix = headers(&[(header::ORIGIN, "https://app.example.com.evil.io")]);
        assert!(!c.origin_ok(&Method::POST, &prefix));
    }

    #[test]
    fn origin_of_and_allowlist() {
        assert_eq!(
            origin_of("https://App.Example.com/admin?x=1").unwrap(),
            "https://app.example.com"
        );
        assert_eq!(
            origin_of("http://localhost:3001").unwrap(),
            "http://localhost:3001"
        );
        assert_eq!(origin_of("not a url"), None);
        assert_eq!(origin_of("https://"), None);

        let prod = allowed_origins(
            "https://app.example.com/",
            Some("https://b.example.com, junk"),
            true,
        );
        assert_eq!(prod, ["https://app.example.com", "https://b.example.com"]);
        let dev = allowed_origins("http://localhost:3001", None, false);
        assert!(dev.contains(&"http://localhost:5173".to_string()));
        // 正式環境不會自動放行 localhost
        assert!(!prod.iter().any(|o| o.contains("localhost")));
    }
}
