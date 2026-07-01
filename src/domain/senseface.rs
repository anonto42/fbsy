use chrono::{FixedOffset, TimeZone};
use sha2::{Digest, Sha256};

use super::event::{parse_utc_offset, HrmsEvent};

#[derive(Debug, Clone)]
pub struct SenseFaceAttendance {
    pub employee_id: String,
    pub event_time: String,
    pub status: String,
    pub verify_mode: String,
    pub work_code: String,
    pub reserved: String,
    pub raw_line: String,
    pub event_key: String,
}

#[derive(Debug, Clone)]
pub struct SenseFaceUser {
    pub employee_id: String,
    pub name: String,
    pub privilege: String,
    pub card: String,
    pub raw_line: String,
}

#[derive(Debug, Clone)]
pub struct PendingForwardAttendance {
    pub id: i64,
    pub serial_number: String,
    pub employee_id: String,
    pub event_time: String,
    pub status: String,
    pub verify_mode: String,
    pub work_code: String,
    pub employee_name: String,
}

pub fn parse_attendance_line(line: &str) -> Option<SenseFaceAttendance> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_prefix = trimmed.strip_prefix("PIN=").unwrap_or(trimmed);
    let parts: Vec<&str> = without_prefix.split('\t').collect();
    if parts.len() < 2 {
        return None;
    }
    let employee_id = parts[0].to_string();
    let event_time = parts[1].to_string();
    if event_time.len() < 16 || !event_time.contains('-') {
        return None;
    }
    let status = parts.get(2).unwrap_or(&"").to_string();
    let verify = parts.get(3).unwrap_or(&"").to_string();
    let work = parts.get(4).unwrap_or(&"").to_string();
    let reserved = parts.get(5).unwrap_or(&"").to_string();
    let event_key = compute_event_key(trimmed);

    Some(SenseFaceAttendance {
        employee_id,
        event_time,
        status,
        verify_mode: verify,
        work_code: work,
        reserved,
        raw_line: trimmed.to_string(),
        event_key,
    })
}

pub fn parse_user_line(line: &str) -> Option<SenseFaceUser> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_user = trimmed
        .strip_prefix("USER ")
        .unwrap_or(trimmed);
    let without_pin = without_user.strip_prefix("PIN=").unwrap_or(without_user);
    let parts: Vec<&str> = without_pin.split('\t').collect();
    if parts.is_empty() {
        return None;
    }
    let employee_id = parts[0].to_string();
    let mut name = String::new();
    let mut privilege = String::new();
    let mut card = String::new();
    for item in &parts[1..] {
        if let Some(value) = item.strip_prefix("Name=") {
            name = value.to_string();
        } else if let Some(value) = item.strip_prefix("Pri=") {
            privilege = value.to_string();
        } else if let Some(value) = item.strip_prefix("Card=") {
            card = value.to_string();
        }
    }

    Some(SenseFaceUser {
        employee_id,
        name,
        privilege,
        card,
        raw_line: trimmed.to_string(),
    })
}

pub fn compute_event_key(raw_line: &str) -> String {
    let hash = Sha256::digest(raw_line.as_bytes());
    format!("{:x}", hash)
}

pub fn compute_request_hash(serial: &str, query: &str, body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(serial.as_bytes());
    hasher.update(b"\0");
    hasher.update(query.as_bytes());
    hasher.update(b"\0");
    hasher.update(body);
    format!("{:x}", hasher.finalize())
}

pub fn event_type_from_adms_status(status: &str) -> &'static str {
    match status {
        "0" => "check_in",
        "" => "check_in",
        _ => "check_out",
    }
}

#[allow(unused_variables)]
pub fn to_hrms_events_from_senseface(
    records: &[PendingForwardAttendance],
    offset: FixedOffset,
    device_code: &str,
    api_key: &str,
    organization_id: u64,
) -> Vec<HrmsEvent> {
    let mut events: Vec<HrmsEvent> = records
        .iter()
        .filter_map(|r| {
            let device_employee_id = r.employee_id.trim();
            if device_employee_id.is_empty() {
                return None;
            }
            let timestamp = parse_adms_timestamp(&r.event_time, offset)?;
            Some(HrmsEvent {
                device_employee_id: device_employee_id.to_string(),
                timestamp,
                event_type: event_type_from_adms_status(&r.status).to_string(),
                verification_method: "senseface".to_string(),
            })
        })
        .collect();

    events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    events
}

fn parse_adms_timestamp(value: &str, offset: FixedOffset) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let naive = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").ok()?;
    offset
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.to_rfc3339())
}

pub fn senseface_timezone_offset(tz: &str) -> Option<FixedOffset> {
    let trimmed = tz.trim();
    if let Some(offset) = parse_utc_offset(trimmed) {
        return Some(offset);
    }
    match trimmed {
        "Asia/Dhaka" | "BDT" => FixedOffset::east_opt(6 * 3600),
        "Asia/Kolkata" | "Asia/Calcutta" | "IST" => FixedOffset::east_opt(5 * 3600 + 30 * 60),
        "Asia/Kathmandu" | "Asia/Katmandu" | "NPT" => FixedOffset::east_opt(5 * 3600 + 45 * 60),
        "Asia/Dubai" | "GST" => FixedOffset::east_opt(4 * 3600),
        "Asia/Bangkok" | "Asia/Ho_Chi_Minh" | "ICT" => FixedOffset::east_opt(7 * 3600),
        "Asia/Singapore" | "Asia/Kuala_Lumpur" | "SGT" | "MYT" => FixedOffset::east_opt(8 * 3600),
        "Asia/Shanghai" | "Asia/Beijing" | "Asia/Hong_Kong" | "CST" => {
            FixedOffset::east_opt(8 * 3600)
        }
        "Asia/Tokyo" | "JST" => FixedOffset::east_opt(9 * 3600),
        "Asia/Seoul" | "KST" => FixedOffset::east_opt(9 * 3600),
        "Asia/Jakarta" | "WIB" => FixedOffset::east_opt(7 * 3600),
        "Asia/Manila" | "PHT" => FixedOffset::east_opt(8 * 3600),
        "Asia/Taipei" | "NST" => FixedOffset::east_opt(8 * 3600),
        "Asia/Riyadh" => FixedOffset::east_opt(3 * 3600),
        "Asia/Karachi" | "PKT" => FixedOffset::east_opt(5 * 3600),
        "Asia/Kabul" | "AFT" => FixedOffset::east_opt(4 * 3600 + 30 * 60),
        "Asia/Tehran" | "IRST" => FixedOffset::east_opt(3 * 3600 + 30 * 60),
        "Asia/Baghdad" => FixedOffset::east_opt(3 * 3600),
        "Asia/Yangon" | "MMT" => FixedOffset::east_opt(6 * 3600 + 30 * 60),
        "Europe/London" | "GMT" | "BST" => FixedOffset::east_opt(0),
        "Europe/Berlin" | "Europe/Paris" | "Europe/Madrid" | "CET" | "CEST" => {
            FixedOffset::east_opt(3600)
        }
        "Europe/Moscow" | "MSK" => FixedOffset::east_opt(3 * 3600),
        "America/New_York" | "EST" | "EDT" => FixedOffset::east_opt(-5 * 3600),
        "America/Chicago" | "CDT" => FixedOffset::east_opt(-6 * 3600),
        "America/Denver" | "MST" | "MDT" => FixedOffset::east_opt(-7 * 3600),
        "America/Los_Angeles" | "PST" | "PDT" => FixedOffset::east_opt(-8 * 3600),
        "America/Sao_Paulo" | "BRT" => FixedOffset::east_opt(-3 * 3600),
        "America/Mexico_City" | "Central Standard Time (Mexico)" => FixedOffset::east_opt(-6 * 3600),
        "Australia/Sydney" | "AEST" | "AEDT" => FixedOffset::east_opt(10 * 3600),
        "Australia/Perth" | "AWST" => FixedOffset::east_opt(8 * 3600),
        "Pacific/Auckland" | "NZST" | "NZDT" => FixedOffset::east_opt(12 * 3600),
        "Africa/Cairo" | "EET" => FixedOffset::east_opt(2 * 3600),
        "Africa/Lagos" | "WAT" => FixedOffset::east_opt(3600),
        "Africa/Johannesburg" | "SAST" => FixedOffset::east_opt(2 * 3600),
        "UTC" | "Z" | "Greenwich Mean Time" => FixedOffset::east_opt(0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_standard_attendance_line() {
        let line = "123\t2026-07-01 09:15:00\t0\t1\t";
        let result = parse_attendance_line(line).unwrap();
        assert_eq!(result.employee_id, "123");
        assert_eq!(result.event_time, "2026-07-01 09:15:00");
        assert_eq!(result.status, "0");
        assert_eq!(result.verify_mode, "1");
    }

    #[test]
    fn parse_attendance_line_with_pin_prefix() {
        let line = "PIN=456\t2026-07-01 10:30:00\t1\t2\tWC1";
        let result = parse_attendance_line(line).unwrap();
        assert_eq!(result.employee_id, "456");
        assert_eq!(result.event_time, "2026-07-01 10:30:00");
        assert_eq!(result.status, "1");
        assert_eq!(result.verify_mode, "2");
        assert_eq!(result.work_code, "WC1");
    }

    #[test]
    fn parse_attendance_line_invalid_returns_none() {
        assert!(parse_attendance_line("").is_none());
        assert!(parse_attendance_line("  ").is_none());
        assert!(parse_attendance_line("singlefield").is_none());
    }

    #[test]
    fn parse_user_line_standard() {
        let line = "USER PIN=789\tName=John\tPri=0\tCard=123456";
        let result = parse_user_line(line).unwrap();
        assert_eq!(result.employee_id, "789");
        assert_eq!(result.name, "John");
        assert_eq!(result.privilege, "0");
        assert_eq!(result.card, "123456");
    }

    #[test]
    fn parse_user_line_without_user_prefix() {
        let line = "PIN=ABC\tName=Jane\tPri=1";
        let result = parse_user_line(line).unwrap();
        assert_eq!(result.employee_id, "ABC");
        assert_eq!(result.name, "Jane");
    }

    #[test]
    fn compute_event_key_is_deterministic() {
        let line = "123\t2026-07-01 09:15:00\t0";
        let key1 = compute_event_key(line);
        let key2 = compute_event_key(line);
        assert_eq!(key1, key2);
        assert_eq!(key1.len(), 64);
    }

    #[test]
    fn event_type_maps_correctly() {
        assert_eq!(event_type_from_adms_status("0"), "check_in");
        assert_eq!(event_type_from_adms_status(""), "check_in");
        assert_eq!(event_type_from_adms_status("1"), "check_out");
        assert_eq!(event_type_from_adms_status("255"), "check_out");
    }

    #[test]
    fn senseface_timezone_parses_fixed_offset() {
        let offset = senseface_timezone_offset("+06:00").unwrap();
        assert_eq!(offset.local_minus_utc(), 6 * 3600);
    }

    #[test]
    fn senseface_timezone_parses_iana_name() {
        let offset = senseface_timezone_offset("Asia/Dhaka").unwrap();
        assert_eq!(offset.local_minus_utc(), 6 * 3600);
    }

    #[test]
    fn senseface_timezone_unknown_returns_none() {
        assert!(senseface_timezone_offset("Mars/Olympus").is_none());
    }

    #[test]
    fn compute_request_hash_is_deterministic() {
        let h1 = compute_request_hash("SN123", "table=ATTLOG", b"data");
        let h2 = compute_request_hash("SN123", "table=ATTLOG", b"data");
        assert_eq!(h1, h2);
    }
}
