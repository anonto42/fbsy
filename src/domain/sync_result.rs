//! Result shape returned by one sync attempt.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// User/API-facing summary of a sync run.
pub struct SyncResult {
    /// Whether the sync completed successfully.
    pub ok: bool,
    /// Device that this result belongs to.
    pub device_code: String,
    /// Number of raw records pulled from the device.
    pub pulled: usize,
    /// Number of events forwarded to HRMS.
    pub forwarded: usize,
    /// Whether device attendance was cleared after successful upload.
    pub device_attendance_cleared: bool,
    /// ISO 8601 UTC timestamp when this sync started (matches Python bridge).
    pub started_at: String,
    /// Human-readable status.
    pub message: String,
}
