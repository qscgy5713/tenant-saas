use crate::common;

use axum::http::{Method, StatusCode, header};
use chrono::{TimeZone, Utc};
use tenant_saas::ical::{Event, calendar, escape_text, fold};

#[test]
fn text_values_are_escaped_so_user_input_cannot_inject_properties() {
    assert_eq!(escape_text("a,b;c\\d"), "a\\,b\\;c\\\\d");
    assert_eq!(escape_text("第一行\r\n第二行"), "第一行\\n第二行");
    // 店名含換行與偽造的屬性:跳脫後只是一行文字,不會變成新屬性
    let evil = "店\nEND:VEVENT\nBEGIN:VEVENT\nATTENDEE:mailto:x@evil";
    let event = Event {
        uid: "u@x",
        sequence: 0,
        starts_at: Utc.with_ymd_and_hms(2026, 10, 5, 2, 0, 0).unwrap(),
        ends_at: Utc.with_ymd_and_hms(2026, 10, 5, 3, 0, 0).unwrap(),
        summary: evil,
        description: "d",
    };
    let ics = calendar(&event, Utc::now());
    // 還原折行後,看「獨立的一行」:偽造的內容只會出現在 SUMMARY 那一行裡面
    let unfolded = ics.replace("\r\n ", "");
    let lines: Vec<&str> = unfolded.split("\r\n").collect();
    assert_eq!(lines.iter().filter(|l| **l == "BEGIN:VEVENT").count(), 1);
    assert_eq!(lines.iter().filter(|l| **l == "END:VEVENT").count(), 1);
    assert!(!lines.iter().any(|l| l.starts_with("ATTENDEE")));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("SUMMARY:店\\nEND:VEVENT"))
    );
}

#[test]
fn long_lines_fold_at_75_bytes_without_splitting_characters() {
    let line = format!("SUMMARY:{}", "預約".repeat(40));
    let folded = fold(&line);
    for part in folded.split("\r\n") {
        assert!(part.len() <= 75, "{} 位元組: {part}", part.len());
    }
    // 還原(去掉折行)後與原文相同,且每個片段都是合法的 UTF-8(能走到這裡就代表沒切壞)
    assert_eq!(folded.replace("\r\n ", ""), line);
    assert_eq!(fold("short"), "short");
}

#[test]
fn calendar_has_required_fields_utc_times_and_crlf() {
    let event = Event {
        uid: "booking-1@tenant-saas",
        sequence: 2,
        starts_at: Utc.with_ymd_and_hms(2026, 10, 5, 2, 0, 0).unwrap(),
        ends_at: Utc.with_ymd_and_hms(2026, 10, 5, 3, 30, 0).unwrap(),
        summary: "剪髮 - 森林系髮廊",
        description: "人員:林美玲",
    };
    let ics = calendar(&event, Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap());
    for needle in [
        "BEGIN:VCALENDAR\r\n",
        "VERSION:2.0\r\n",
        "UID:booking-1@tenant-saas\r\n",
        "SEQUENCE:2\r\n",
        "DTSTAMP:20261001T000000Z\r\n",
        "DTSTART:20261005T020000Z\r\n",
        "DTEND:20261005T033000Z\r\n",
        "SUMMARY:剪髮 - 森林系髮廊\r\n",
        "END:VCALENDAR\r\n",
    ] {
        assert!(ics.contains(needle), "缺少 {needle:?}\n{ics}");
    }
    assert!(!ics.replace("\r\n", "").contains('\n'), "行尾一律 CRLF");
}

#[sqlx::test]
async fn customers_download_an_ics_only_for_confirmed_bookings(pool: sqlx::PgPool) {
    use crate::common::{add_member, app, call, create_tenant, signup};
    use serde_json::json;
    let app = app(pool.clone());
    let owner = signup(&app, "o@example.com").await;
    create_tenant(&app, &owner, "shop-a").await;
    let _ = add_member;
    let svc = common::create_service(&app, &owner, "shop-a", "剪髮").await;
    let me = common::user_id(&pool, "o@example.com").await;
    call(
        &app,
        Method::PUT,
        &format!("/t/shop-a/members/{me}/services"),
        Some(json!({"service_ids": [svc["id"]]})),
        Some(&owner),
    )
    .await;
    call(&app, Method::PUT, &format!("/t/shop-a/members/{me}/working-hours"), Some(json!({"hours": (0..7).map(|d| json!({"weekday": d, "start": "00:00", "end": "23:59"})).collect::<Vec<_>>()})), Some(&owner)).await;

    let start = (Utc::now() + chrono::Duration::days(3))
        .format("%Y-%m-%dT10:00:00Z")
        .to_string();
    let (status, created) = call(&app, Method::POST, "/t/shop-a/bookings", Some(json!({"service_id": svc["id"], "start": start, "customer": {"name": "客", "email": "c@example.com"}})), Some(&owner)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let token = created["manage_token"].as_str().unwrap();

    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/public/bookings/{token}/calendar.ics"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        res.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/calendar")
    );
    assert!(
        res.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("booking.ics")
    );
    assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
    let body =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(body.contains("SUMMARY:") && body.contains("SEQUENCE:0"));
    let uid = body
        .lines()
        .find(|l| l.starts_with("UID:"))
        .unwrap()
        .to_string();

    // 改期後同一個 UID、SEQUENCE 加一 → 行事曆會更新同一個事件
    let new_start = (Utc::now() + chrono::Duration::days(4))
        .format("%Y-%m-%dT11:00:00Z")
        .to_string();
    call(
        &app,
        Method::POST,
        &format!("/public/bookings/{token}/reschedule"),
        Some(json!({"start": new_start})),
        None,
    )
    .await;
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/public/bookings/{token}/calendar.ics"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body2 =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(body2.contains("SEQUENCE:1"));
    assert_eq!(body2.lines().find(|l| l.starts_with("UID:")).unwrap(), uid);

    // 無效 token → 404;已取消 → 409
    let (s404, _) = call(
        &app,
        Method::GET,
        &format!("/public/bookings/{}/calendar.ics", "z".repeat(64)),
        None,
        None,
    )
    .await;
    assert_eq!(s404, StatusCode::NOT_FOUND);
    call(
        &app,
        Method::POST,
        &format!("/public/bookings/{token}/cancel"),
        None,
        None,
    )
    .await;
    let (s409, _) = call(
        &app,
        Method::GET,
        &format!("/public/bookings/{token}/calendar.ics"),
        None,
        None,
    )
    .await;
    assert_eq!(s409, StatusCode::CONFLICT);
}
