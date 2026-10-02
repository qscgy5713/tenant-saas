//! 產生 iCalendar(RFC 5545)事件,讓顧客把預約加進自己的行事曆。

use chrono::{DateTime, Utc};

pub struct Event<'a> {
    /// 全域唯一且**不變**的識別碼:行事曆靠它判斷「這是同一個事件的更新」,不是新事件
    pub uid: &'a str,
    /// 每次修改(改期)加一,行事曆才會用新版本取代舊的
    pub sequence: i32,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub summary: &'a str,
    pub description: &'a str,
}

/// TEXT 值的跳脫:反斜線、分號、逗號、換行(RFC 5545 §3.3.11)。
/// 使用者可控的文字(店名、服務名)絕不能原樣放進去,否則換行就能注入任意屬性。
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// 每行最多 75 個**位元組**,超過就折行(CRLF + 一個空白)。不可從 UTF-8 字元中間切開
pub fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut len = 0;
    for c in line.chars() {
        let w = c.len_utf8();
        // 續行開頭有一個空白,已算進 len
        if len + w > 75 {
            out.push_str("\r\n ");
            len = 1;
        }
        out.push(c);
        len += w;
    }
    out
}

fn stamp(t: DateTime<Utc>) -> String {
    t.format("%Y%m%dT%H%M%SZ").to_string()
}

/// 完整的 .ics 內容(行尾一律 CRLF)
pub fn calendar(event: &Event, now: DateTime<Utc>) -> String {
    let lines = [
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//tenant-saas//booking//ZH".to_string(),
        "CALSCALE:GREGORIAN".to_string(),
        "METHOD:PUBLISH".to_string(),
        "BEGIN:VEVENT".to_string(),
        format!("UID:{}", escape_text(event.uid)),
        format!("SEQUENCE:{}", event.sequence),
        format!("DTSTAMP:{}", stamp(now)),
        format!("DTSTART:{}", stamp(event.starts_at)),
        format!("DTEND:{}", stamp(event.ends_at)),
        format!("SUMMARY:{}", escape_text(event.summary)),
        format!("DESCRIPTION:{}", escape_text(event.description)),
        "STATUS:CONFIRMED".to_string(),
        "END:VEVENT".to_string(),
        "END:VCALENDAR".to_string(),
    ];
    let mut out = String::new();
    for line in lines {
        out.push_str(&fold(&line));
        out.push_str("\r\n");
    }
    out
}
