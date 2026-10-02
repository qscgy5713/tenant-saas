//! 可預約時段計算(純運算,不碰資料庫,方便單元測試)。
//!
//! 營業時間以「店家本地時間」儲存,這裡依店家時區換算成 UTC 絕對時間:
//! - 夏令時間跳時造成的「不存在的時刻」:該段營業時間邊界無法換算,整段略過
//! - 回撥造成的「重複的時刻」:取較早的那一個

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Serialize;
use uuid::Uuid;

/// 時段起點的間隔(分鐘)。起點對齊每段營業時間的開始時間。
pub const SLOT_STEP_MINUTES: i64 = 15;

#[derive(Debug, Clone)]
pub struct StaffSchedule {
    pub user_id: Uuid,
    /// (weekday: 0 = 週日, 開始, 結束),皆為店家本地時間
    pub hours: Vec<(u32, NaiveTime, NaiveTime)>,
    /// 不可預約的絕對時間區間(休假與既有預約),半開區間 [start, end)
    pub busy: Vec<(DateTime<Utc>, DateTime<Utc>)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Slot {
    pub staff_id: Uuid,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

pub fn local_to_utc(tz: Tz, date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    tz.from_local_datetime(&date.and_time(time))
        .earliest()
        .map(|d| d.with_timezone(&Utc))
}

/// 計算 [from, to](店家本地日期,含頭尾)內所有可預約的時段,依時間與員工排序。
/// 起點必須晚於 `now`。
pub fn compute_slots(
    tz: Tz,
    from: NaiveDate,
    to: NaiveDate,
    duration: Duration,
    step: Duration,
    now: DateTime<Utc>,
    schedules: &[StaffSchedule],
) -> Vec<Slot> {
    let mut slots = Vec::new();
    let mut date = from;
    while date <= to {
        let weekday = date.weekday().num_days_from_sunday();
        for schedule in schedules {
            for &(wd, start, end) in &schedule.hours {
                if wd != weekday {
                    continue;
                }
                let (Some(window_start), Some(window_end)) =
                    (local_to_utc(tz, date, start), local_to_utc(tz, date, end))
                else {
                    continue;
                };
                let mut cursor = window_start;
                while cursor + duration <= window_end {
                    let slot_end = cursor + duration;
                    let free = !schedule
                        .busy
                        .iter()
                        .any(|&(b_start, b_end)| b_start < slot_end && b_end > cursor);
                    if cursor > now && free {
                        slots.push(Slot {
                            staff_id: schedule.user_id,
                            start: cursor,
                            end: slot_end,
                        });
                    }
                    cursor += step;
                }
            }
        }
        date = match date.succ_opt() {
            Some(d) => d,
            None => break,
        };
    }
    slots.sort_by_key(|s| (s.start, s.staff_id));
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }
    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }
    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }
    fn staff(
        hours: Vec<(u32, NaiveTime, NaiveTime)>,
        busy: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    ) -> StaffSchedule {
        StaffSchedule {
            user_id: Uuid::nil(),
            hours,
            busy,
        }
    }
    const LONG_AGO: &str = "2020-01-01T00:00:00Z";
    const TAIPEI: Tz = chrono_tz::Asia::Taipei;

    #[test]
    fn basic_grid() {
        // 2026-10-05 是週一;09:00–12:00,60 分鐘服務,每 15 分鐘一個起點 → 09:00 … 11:00 共 9 個
        let s = staff(vec![(1, t(9, 0), t(12, 0))], vec![]);
        let slots = compute_slots(
            TAIPEI,
            d(2026, 10, 5),
            d(2026, 10, 5),
            Duration::minutes(60),
            Duration::minutes(15),
            utc(LONG_AGO),
            &[s],
        );
        assert_eq!(slots.len(), 9);
        assert_eq!(slots[0].start, utc("2026-10-05T01:00:00Z")); // 台北 09:00 = UTC 01:00
        assert_eq!(slots[8].end, utc("2026-10-05T04:00:00Z"));
    }

    #[test]
    fn other_weekdays_have_no_slots() {
        let s = staff(vec![(1, t(9, 0), t(12, 0))], vec![]);
        let slots = compute_slots(
            TAIPEI,
            d(2026, 10, 6),
            d(2026, 10, 6),
            Duration::minutes(30),
            Duration::minutes(15),
            utc(LONG_AGO),
            &[s],
        );
        assert!(slots.is_empty());
    }

    #[test]
    fn busy_intervals_block_overlapping_slots_but_not_adjacent_ones() {
        // 台北 10:00–11:00 已被占用(UTC 02:00–03:00)
        let busy = vec![(utc("2026-10-05T02:00:00Z"), utc("2026-10-05T03:00:00Z"))];
        let s = staff(vec![(1, t(9, 0), t(12, 0))], busy);
        let slots = compute_slots(
            TAIPEI,
            d(2026, 10, 5),
            d(2026, 10, 5),
            Duration::minutes(60),
            Duration::minutes(15),
            utc(LONG_AGO),
            &[s],
        );
        let starts: Vec<_> = slots.iter().map(|s| s.start).collect();
        assert_eq!(
            starts,
            vec![utc("2026-10-05T01:00:00Z"), utc("2026-10-05T03:00:00Z")],
            "只剩 09:00(剛好接在前面)與 11:00(剛好接在後面)"
        );
    }

    #[test]
    fn past_slots_are_excluded() {
        let s = staff(vec![(1, t(9, 0), t(12, 0))], vec![]);
        // 現在是台北 10:00:現在這一刻本身不算,必須嚴格晚於
        let slots = compute_slots(
            TAIPEI,
            d(2026, 10, 5),
            d(2026, 10, 5),
            Duration::minutes(60),
            Duration::minutes(15),
            utc("2026-10-05T02:00:00Z"),
            &[s],
        );
        assert_eq!(slots[0].start, utc("2026-10-05T02:15:00Z"));
    }

    #[test]
    fn multiple_windows_and_staff_are_sorted() {
        let mut a = staff(
            vec![(1, t(13, 0), t(14, 0)), (1, t(9, 0), t(10, 0))],
            vec![],
        );
        a.user_id = Uuid::from_u128(2);
        let mut b = staff(vec![(1, t(9, 0), t(10, 0))], vec![]);
        b.user_id = Uuid::from_u128(1);
        let slots = compute_slots(
            TAIPEI,
            d(2026, 10, 5),
            d(2026, 10, 5),
            Duration::minutes(60),
            Duration::minutes(60),
            utc(LONG_AGO),
            &[a, b],
        );
        let order: Vec<_> = slots
            .iter()
            .map(|s| (s.start, s.staff_id.as_u128()))
            .collect();
        assert_eq!(
            order,
            vec![
                (utc("2026-10-05T01:00:00Z"), 1),
                (utc("2026-10-05T01:00:00Z"), 2),
                (utc("2026-10-05T05:00:00Z"), 2),
            ]
        );
    }

    #[test]
    fn dst_spring_forward_gap_is_skipped() {
        // 紐約 2026-03-08(週日)02:00 跳到 03:00,02:00–04:00 的起點不存在 → 整段略過
        let ny = chrono_tz::America::New_York;
        let s = staff(vec![(0, t(2, 0), t(4, 0))], vec![]);
        let slots = compute_slots(
            ny,
            d(2026, 3, 8),
            d(2026, 3, 8),
            Duration::minutes(30),
            Duration::minutes(30),
            utc(LONG_AGO),
            &[s],
        );
        assert!(slots.is_empty());
    }

    #[test]
    fn dst_spring_forward_window_is_one_hour_shorter() {
        // 01:00 EST (06:00Z) → 03:30 EDT (07:30Z),實際只有 1.5 小時,30 分鐘服務 → 3 個起點
        let ny = chrono_tz::America::New_York;
        let s = staff(vec![(0, t(1, 0), t(3, 30))], vec![]);
        let slots = compute_slots(
            ny,
            d(2026, 3, 8),
            d(2026, 3, 8),
            Duration::minutes(30),
            Duration::minutes(30),
            utc(LONG_AGO),
            &[s],
        );
        let starts: Vec<_> = slots.iter().map(|s| s.start).collect();
        assert_eq!(
            starts,
            vec![
                utc("2026-03-08T06:00:00Z"),
                utc("2026-03-08T06:30:00Z"),
                utc("2026-03-08T07:00:00Z")
            ]
        );
    }

    #[test]
    fn dst_fall_back_ambiguous_start_takes_the_earlier_instant() {
        // 紐約 2026-11-01(週日)01:00 出現兩次;起點取較早的 EDT (05:00Z),結束 02:00 EST = 07:00Z → 實際 2 小時
        let ny = chrono_tz::America::New_York;
        let s = staff(vec![(0, t(1, 0), t(2, 0))], vec![]);
        let slots = compute_slots(
            ny,
            d(2026, 11, 1),
            d(2026, 11, 1),
            Duration::minutes(60),
            Duration::minutes(60),
            utc(LONG_AGO),
            &[s],
        );
        let starts: Vec<_> = slots.iter().map(|s| s.start).collect();
        assert_eq!(
            starts,
            vec![utc("2026-11-01T05:00:00Z"), utc("2026-11-01T06:00:00Z")]
        );
    }

    #[test]
    fn date_range_spans_multiple_days() {
        let s = staff(vec![(1, t(9, 0), t(10, 0)), (2, t(9, 0), t(10, 0))], vec![]);
        let slots = compute_slots(
            TAIPEI,
            d(2026, 10, 5),
            d(2026, 10, 7),
            Duration::minutes(60),
            Duration::minutes(60),
            utc(LONG_AGO),
            &[s],
        );
        assert_eq!(slots.len(), 2, "週一、週二各一個,週三沒有營業");
    }
}
