use chrono::FixedOffset;
use fingerbridge::domain::{
    default_utc_offset, event_type_from_punch, parse_timestamp, parse_utc_offset,
    resolve_device_timezone_offset, resolve_iana_timezone_offset, to_hrms_events, EventTypeMode,
    RawAttendance,
};
use serde_json::json;

fn utc() -> FixedOffset {
    default_utc_offset()
}

fn plus_six() -> FixedOffset {
    parse_utc_offset("+06:00").expect("offset parses")
}

#[test]
fn punch_codes_match_python_bridge_behavior() {
    assert_eq!(event_type_from_punch(0), "check_in");
    assert_eq!(event_type_from_punch(4), "check_in");
    assert_eq!(event_type_from_punch(1), "check_out");
    assert_eq!(event_type_from_punch(99), "check_out");
}

#[test]
fn naive_timestamp_uses_utc_when_no_offset_configured() {
    let parsed = parse_timestamp("2026-05-21T10:15:00", utc()).expect("timestamp parses");
    assert_eq!(parsed, "2026-05-21T10:15:00+00:00");
}

#[test]
fn naive_timestamp_uses_configured_device_offset() {
    // A device in +06:00 reporting 10:15 wall-clock is 10:15+06:00, not 10:15Z.
    let parsed = parse_timestamp("2026-05-21T10:15:00", plus_six()).expect("timestamp parses");
    assert_eq!(parsed, "2026-05-21T10:15:00+06:00");
}

#[test]
fn aware_timestamp_keeps_its_offset_regardless_of_config() {
    let parsed = parse_timestamp("2026-05-21T12:00:00+06:00", utc()).expect("timestamp parses");
    assert_eq!(parsed, "2026-05-21T12:00:00+06:00");
}

#[test]
fn invalid_timestamp_returns_none() {
    assert_eq!(parse_timestamp("not-a-date", utc()), None);
}

#[test]
fn utc_offset_parser_accepts_common_forms() {
    assert_eq!(parse_utc_offset(""), Some(utc()));
    assert_eq!(parse_utc_offset("UTC"), Some(utc()));
    assert_eq!(parse_utc_offset("Z"), Some(utc()));
    assert_eq!(parse_utc_offset("+06:00"), FixedOffset::east_opt(6 * 3600));
    assert_eq!(parse_utc_offset("+0600"), FixedOffset::east_opt(6 * 3600));
    assert_eq!(parse_utc_offset("+06"), FixedOffset::east_opt(6 * 3600));
    assert_eq!(
        parse_utc_offset("-05:30"),
        FixedOffset::east_opt(-(5 * 3600 + 30 * 60))
    );
    assert_eq!(parse_utc_offset("garbage"), None);
    assert_eq!(parse_utc_offset("+25:00"), None);
}

#[test]
fn device_timezone_accepts_iana_name() {
    assert_eq!(
        resolve_device_timezone_offset("Asia/Dhaka"),
        FixedOffset::east_opt(6 * 3600)
    );
    assert_eq!(
        resolve_device_timezone_offset("+06:00"),
        FixedOffset::east_opt(6 * 3600)
    );
    assert_eq!(resolve_device_timezone_offset("Not/ARealZone"), None);
}

#[test]
fn hrms_events_skip_malformed_records_and_sort_by_timestamp() {
    let events = to_hrms_events(
        &[
            RawAttendance {
                user_id: "42".to_string(),
                timestamp: "2026-05-21T10:20:00".to_string(),
                punch: 1,
            },
            RawAttendance {
                user_id: "".to_string(),
                timestamp: "2026-05-21T10:10:00".to_string(),
                punch: 0,
            },
            RawAttendance {
                user_id: "11".to_string(),
                timestamp: "not-a-date".to_string(),
                punch: 0,
            },
            RawAttendance {
                user_id: "7".to_string(),
                timestamp: "2026-05-21T10:05:00".to_string(),
                punch: 0,
            },
        ],
        utc(),
        EventTypeMode::PunchCode,
    );

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].device_employee_id, "7");
    assert_eq!(events[0].event_type, "check_in");
    assert_eq!(events[1].device_employee_id, "42");
    assert_eq!(events[1].event_type, "check_out");
}

#[test]
fn event_serializes_to_exact_hrms_field_names() {
    let event = to_hrms_events(
        &[RawAttendance {
            user_id: "001".to_string(),
            timestamp: "2026-06-25T02:30:00Z".to_string(),
            punch: 0,
        }],
        utc(),
        EventTypeMode::PunchCode,
    )
    .into_iter()
    .next()
    .expect("event");

    let value = serde_json::to_value(event).expect("serialize event");
    assert_eq!(
        value,
        json!({
            "deviceEmployeeId": "001",
            "timestamp": "2026-06-25T02:30:00+00:00",
            "eventType": "check_in",
            "verificationMethod": "fingerbridge"
        })
    );
}

#[test]
fn first_in_last_out_mode_ignores_punch_codes_per_employee_day() {
    let events = to_hrms_events(
        &[
            RawAttendance {
                user_id: "7".to_string(),
                timestamp: "2026-07-09T18:05:00".to_string(),
                punch: 0,
            },
            RawAttendance {
                user_id: "7".to_string(),
                timestamp: "2026-07-09T08:55:00".to_string(),
                punch: 0,
            },
            RawAttendance {
                user_id: "8".to_string(),
                timestamp: "2026-07-09T09:00:00".to_string(),
                punch: 4,
            },
            RawAttendance {
                user_id: "7".to_string(),
                timestamp: "2026-07-10T08:58:00".to_string(),
                punch: 0,
            },
        ],
        plus_six(),
        EventTypeMode::FirstInLastOut,
    );

    assert_eq!(events[0].device_employee_id, "7");
    assert_eq!(events[0].event_type, "check_in");
    assert_eq!(events[1].device_employee_id, "8");
    assert_eq!(events[1].event_type, "check_in");
    assert_eq!(events[2].device_employee_id, "7");
    assert_eq!(events[2].event_type, "check_out");
    assert_eq!(events[3].device_employee_id, "7");
    assert_eq!(events[3].event_type, "check_in");
}

#[test]
fn resolve_iana_timezone_offset_recognizes_asia_dhaka() {
    let offset = resolve_iana_timezone_offset("Asia/Dhaka").expect("known IANA name");
    assert_eq!(offset, FixedOffset::east_opt(6 * 3600).unwrap());
}

#[test]
fn resolve_iana_timezone_offset_rejects_unknown_name() {
    assert!(resolve_iana_timezone_offset("Not/ARealZone").is_none());
}

#[test]
fn resolve_iana_timezone_offset_trims_whitespace() {
    let offset = resolve_iana_timezone_offset("  Asia/Dhaka  ").expect("trims whitespace");
    assert_eq!(offset, FixedOffset::east_opt(6 * 3600).unwrap());
}
