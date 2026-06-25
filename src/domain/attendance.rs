//! Raw attendance data pulled from a ZKTeco device.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
/// Minimal raw attendance record used by the sync engine.
pub struct RawAttendance {
    /// Employee/user identifier stored on the device.
    pub user_id: String,
    /// Timestamp as reported by the device adapter.
    pub timestamp: String,
    /// ZKTeco punch code. The bridge maps this into check-in/check-out.
    pub punch: i64,
}
