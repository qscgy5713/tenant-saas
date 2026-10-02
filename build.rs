// sqlx::migrate! 在編譯時把 migrations/ 嵌進程式。Cargo 預設不會因為資料夾裡多了檔案而重新編譯,
// 單獨新增一個 migration 檔就會悄悄用到舊版本(程式回報「migration 完成」,實際沒套用)。
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
