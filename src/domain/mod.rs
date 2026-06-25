//! Domain model.
//!
//! Domain code should stay pure: no CLI parsing, no filesystem access, no HTTP
//! client, and no device protocol. That makes it easy to test and reason about.

pub mod attendance;
pub mod event;
pub mod sync_result;
pub mod template;

pub use attendance::RawAttendance;
pub use event::{event_type_from_punch, parse_timestamp, to_hrms_events, HrmsEvent};
pub use sync_result::SyncResult;
pub use template::{DeviceUser, FingerTemplate, FingerTemplatePayload};
