//! 產生 CSV(RFC 4180),給「匯出」用。
//!
//! 兩件事一定要做對:
//! 1. **試算表公式注入**:欄位開頭是 `=` `+` `-` `@`(或 Tab / 換行字元),Excel / Google 試算表會把它當公式執行
//!    (例如 `=HYPERLINK("http://evil",…)` 或更糟)。匯出的資料裡有使用者自己填的文字(姓名、備註),
//!    任何人註冊時都能把姓名設成公式,管理者匯出後一開檔就中招。對策:開頭加上單引號,讓它被當成純文字。
//! 2. 含逗號、雙引號、換行的欄位要用雙引號包起來,雙引號本身要重複一次。

/// 一次匯出的筆數上限。超過就請對方縮小範圍,而不是默默截斷(截斷的資料比沒有更糟:看起來完整,實際缺了)。
/// 整個檔案會先在記憶體裡組好,所以才需要上限(50,000 筆約 10 MB)
pub const EXPORT_MAX_ROWS: i64 = 50_000;

/// UTF-8 BOM:Excel 開啟沒有 BOM 的 UTF-8 檔案會把中文當成亂碼
pub const BOM: &str = "\u{feff}";

/// 把一個欄位轉成安全的 CSV 欄位(含公式注入防護與必要的引號)
pub fn field(value: &str) -> String {
    let dangerous = value
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@' | '\t' | '\r' | '\n'));
    let mut text = String::with_capacity(value.len() + 2);
    if dangerous {
        text.push('\'');
    }
    text.push_str(value);

    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// 一列(結尾是 CRLF,RFC 4180 的規定)
pub fn row<I, S>(fields: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut line = fields
        .into_iter()
        .map(|f| field(f.as_ref()))
        .collect::<Vec<_>>()
        .join(",");
    line.push_str("\r\n");
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_fields_are_left_alone() {
        assert_eq!(field("booking.cancelled"), "booking.cancelled");
        assert_eq!(field("王小明"), "王小明");
        assert_eq!(field(""), "");
        assert_eq!(field("2026-10-06 10:00:00"), "2026-10-06 10:00:00");
    }

    #[test]
    fn commas_quotes_and_newlines_are_quoted() {
        assert_eq!(field("a,b"), "\"a,b\"");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(field("line1\nline2"), "\"line1\nline2\"");
        assert_eq!(field("a\r\nb"), "\"a\r\nb\"");
        assert_eq!(
            field(r#"{"from":"pending","to":"cancelled"}"#),
            r#""{""from"":""pending"",""to"":""cancelled""}""#
        );
    }

    #[test]
    fn spreadsheet_formulas_are_neutralised() {
        for evil in [
            "=1+1",
            "+cmd|' /C calc'!A0",
            "-2+3",
            "@SUM(A1:A9)",
            "=HYPERLINK(\"http://evil\",\"x\")",
            "\t=1+1",
            "\r=1+1",
        ] {
            let out = field(evil);
            assert!(
                out.starts_with('\'') || out.starts_with("\"'"),
                "{evil:?} 要被加上單引號: {out}"
            );
        }
        // 加了單引號之後如果含逗號 / 引號,仍然要正確包起來
        assert_eq!(field("=A1,B1"), "\"'=A1,B1\"");
        // 只有「開頭」危險:中間出現 = 不用處理
        assert_eq!(field("a=b"), "a=b");
        assert_eq!(field("王=小明"), "王=小明");
    }

    #[test]
    fn rows_end_with_crlf_and_fields_are_independent() {
        assert_eq!(row(["a", "b,c", "=x"]), "a,\"b,c\",'=x\r\n");
        assert_eq!(row(Vec::<String>::new()), "\r\n");
    }
}
