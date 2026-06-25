//! Conversion from device attendance records to HRMS webhook events.

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use super::RawAttendance;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
/// Event shape expected by the HRMS webhook.
pub struct HrmsEvent {
    /// Employee identifier in the HRMS payload.
    pub device_employee_id: String,
    /// Offset-aware ISO 8601 timestamp passed to HRMS.
    pub timestamp: String,
    /// Normalized attendance event type.
    pub event_type: String,
    /// Source marker that tells HRMS where the event came from.
    pub verification_method: String,
}

/// Parse a device timestamp into the ISO string sent to HRMS.
///
/// The Python bridge treats naive device timestamps as UTC. We preserve that
/// behavior until real-device timezone handling is confirmed.
pub fn parse_timestamp(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if let Ok(timestamp) = DateTime::parse_from_rfc3339(value) {
        return Some(timestamp.to_rfc3339());
    }

    parse_naive_timestamp(value).map(|timestamp| Utc.from_utc_datetime(&timestamp).to_rfc3339())
}

/// Map ZKTeco punch codes into the HRMS event vocabulary.
pub fn event_type_from_punch(punch: i64) -> &'static str {
    match punch {
        // The Python bridge treats punch codes 0 and 4 as check-in.
        0 | 4 => "check_in",
        // Every other punch code is treated as check-out for compatibility.
        _ => "check_out",
    }
}

/// Convert raw device records into sorted HRMS events.
pub fn to_hrms_events(records: &[RawAttendance]) -> Vec<HrmsEvent> {
    let mut events = records
        .iter()
        .filter_map(to_hrms_event)
        .collect::<Vec<HrmsEvent>>();

    // Stable chronological ordering makes webhook behavior predictable.
    events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    events
}

fn to_hrms_event(record: &RawAttendance) -> Option<HrmsEvent> {
    let device_employee_id = record.user_id.trim();
    if device_employee_id.is_empty() {
        return None;
    }

    Some(HrmsEvent {
        device_employee_id: device_employee_id.to_string(),
        timestamp: parse_timestamp(&record.timestamp)?,
        event_type: event_type_from_punch(record.punch).to_string(),
        verification_method: "zkteco_bridge".to_string(),
    })
}

fn parse_naive_timestamp(value: &str) -> Option<NaiveDateTime> {
    const FORMATS: &[&str] = &[
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
    ];

    FORMATS
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
}

#[allow(dead_code)]
fn _fixed_offset_type_marker(_: DateTime<FixedOffset>) {}
