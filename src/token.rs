use std::fmt::Write as _;

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// 一次性連結用的隨機 token:兩個 UUIDv4 串接(約 244 位元隨機),十六進位 64 字元
pub fn generate() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// 資料庫只存雜湊,原始 token 只在建立時回傳一次
pub fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}
