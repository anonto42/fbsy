//! Conversion from device attendance records to HRMS webhook events.

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
/// Strategy used to derive HRMS check-in/check-out from device punches.
pub enum EventTypeMode {
    /// Trust the ZKTeco punch code: 0/4 = check_in, everything else = check_out.
    PunchCode,
    /// Ignore punch direction and derive first-in/last-out per employee/day.
    FirstInLastOut,
}

impl Default for EventTypeMode {
    fn default() -> Self {
        Self::PunchCode
    }
}

impl EventTypeMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "punchcode" | "punch_code" | "punch-code" => Some(Self::PunchCode),
            "firstinlastout" | "first_in_last_out" | "first-in-last-out" => {
                Some(Self::FirstInLastOut)
            }
            _ => None,
        }
    }
}

/// Parse a configured device timezone string into a fixed UTC offset.
///
/// ZKTeco devices report **naive** wall-clock timestamps with no offset, so the
/// bridge must be told which offset that wall-clock is in to map a punch onto the
/// correct calendar instant. Accepted forms (case-insensitive):
///
/// - empty / `UTC` / `Z` → `+00:00`
/// - `+06:00`, `-05:30`, `+0600`, `-0530`, `+06`, `-05`
///
/// Returns `None` if the string cannot be understood (the caller treats that as a
/// validation error).
pub fn parse_utc_offset(value: &str) -> Option<FixedOffset> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("utc")
        || trimmed.eq_ignore_ascii_case("z")
    {
        return FixedOffset::east_opt(0);
    }

    let (sign, rest) = match trimmed.as_bytes().first()? {
        b'+' => (1, &trimmed[1..]),
        b'-' => (-1, &trimmed[1..]),
        _ => return None,
    };

    // Accept "HH", "HHMM", "HH:MM".
    let (hours_str, minutes_str) = if let Some((h, m)) = rest.split_once(':') {
        (h, m)
    } else if rest.len() <= 2 {
        (rest, "0")
    } else if rest.len() == 4 {
        (&rest[..2], &rest[2..])
    } else {
        return None;
    };

    let hours: i32 = hours_str.parse().ok()?;
    let minutes: i32 = minutes_str.parse().ok()?;
    if !(0..=23).contains(&hours) || !(0..=59).contains(&minutes) {
        return None;
    }

    FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60))
}

/// The UTC offset used when a device has no configured timezone.
pub fn default_utc_offset() -> FixedOffset {
    FixedOffset::east_opt(0).expect("zero offset is always valid")
}

/// Resolve an IANA timezone name (e.g. `"Asia/Dhaka"`, as sent by HRMS in
/// `orgTimezone`) into the fixed UTC offset currently in effect for it.
///
/// This intentionally resolves against *today's* date rather than caching a
/// single offset forever, so a timezone with DST still gets the correct
/// offset for whenever the bridge actually starts using it — even though
/// individual sync cycles then apply that one resolved offset uniformly
/// (matching this bridge's fixed-offset-per-sync design; see
/// `parse_utc_offset` for why full IANA/DST-aware parsing isn't used
/// per-timestamp).
///
/// Returns `None` if the name isn't a recognized IANA timezone.
pub fn resolve_iana_timezone_offset(name: &str) -> Option<FixedOffset> {
    use chrono::Offset;

    let tz: chrono_tz::Tz = name.trim().parse().ok()?;
    let now = Utc::now().with_timezone(&tz);
    Some(now.offset().fix())
}

/// Resolve a configured device timezone into the fixed offset to use for this
/// sync cycle. Accepts fixed offsets (`+06:00`) and IANA names (`Asia/Dhaka`).
pub fn resolve_device_timezone_offset(value: &str) -> Option<FixedOffset> {
    parse_utc_offset(value).or_else(|| resolve_iana_timezone_offset(value))
}

/// Parse a device timestamp into the offset-aware ISO string sent to HRMS.
///
/// Naive device timestamps are interpreted as wall-clock time in `offset` (the
/// device's configured timezone). Timestamps that already carry an offset keep it.
pub fn parse_timestamp(value: &str, offset: FixedOffset) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if let Ok(timestamp) = DateTime::parse_from_rfc3339(value) {
        return Some(timestamp.to_rfc3339());
    }

    parse_naive_timestamp(value).and_then(|naive| {
        offset
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.to_rfc3339())
    })
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

/// Convert raw device records into sorted HRMS events using the device's offset.
pub fn to_hrms_events(
    records: &[RawAttendance],
    offset: FixedOffset,
    event_type_mode: EventTypeMode,
) -> Vec<HrmsEvent> {
    let mut events = records
        .iter()
        .filter_map(|record| to_hrms_event(record, offset))
        .collect::<Vec<HrmsEvent>>();

    // Stable chronological ordering makes webhook behavior predictable.
    events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    if event_type_mode == EventTypeMode::FirstInLastOut {
        apply_first_in_last_out(&mut events);
    }
    events
}

fn apply_first_in_last_out(events: &mut [HrmsEvent]) {
    let mut seen = HashSet::new();
    for event in events {
        let local_date = event.timestamp.get(..10).unwrap_or("");
        let key = (event.device_employee_id.clone(), local_date.to_string());
        event.event_type = if seen.insert(key) {
            "check_in".to_string()
        } else {
            "check_out".to_string()
        };
    }
}

fn to_hrms_event(record: &RawAttendance, offset: FixedOffset) -> Option<HrmsEvent> {
    let device_employee_id = record.user_id.trim();
    if device_employee_id.is_empty() {
        return None;
    }

    Some(HrmsEvent {
        device_employee_id: device_employee_id.to_string(),
        timestamp: parse_timestamp(&record.timestamp, offset)?,
        event_type: event_type_from_punch(record.punch).to_string(),
        verification_method: "fingerbridge".to_string(),
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
