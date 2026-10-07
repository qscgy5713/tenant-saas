//! 所有整合測試編成**同一個**執行檔(每個檔案一個模組)。
//!
//! 為什麼:`tests/` 底下每個檔案原本各自是一個執行檔,每個都要把整個專案與相依套件連結一次;
//! 28 個檔案 = 28 次連結,改一行程式碼完整檢查就要十幾分鐘。合併後只連結一次。
//! 跑單一檔案的測試:`cargo test --test it booking::`。
//!
//! `restricted_role.rs` 刻意維持獨立:它是 `--ignored` 的測試,只在權限受限的資料庫上跑(CI 的專用工作)。
#[path = "../common/mod.rs"]
mod common;

mod account_deletion;
mod account_events;
mod admin_cli;
mod audit;
mod auth;
mod billing;
mod booking;
mod buffer;
mod cookie_auth;
mod customers;
mod data_export;
mod email_change;
mod email_verification;
mod ical;
mod input_hygiene;
mod lockout;
mod mail_quota;
mod members;
mod observability;
mod password_reset;
mod plans;
mod retention;
mod scheduling;
mod shop_limit;
mod shop_profile;
mod tenancy;
mod worker;
