//! Domain model.
//!
//! Domain code should stay pure: no CLI parsing, no filesystem access, no HTTP
//! client, and no device protocol. That makes it easy to test and reason about.

pub mod attendance;
pub mod event;
pub mod senseface;
pub mod sync_result;
pub mod template;

pub use attendance::RawAttendance;
pub use event::{
    default_utc_offset, event_type_from_punch, parse_timestamp, parse_utc_offset,
    resolve_iana_timezone_offset, to_hrms_events, HrmsEvent,
};
pub use sync_result::SyncResult;
pub use template::{DeviceUser, FingerTemplate, FingerTemplatePayload};
