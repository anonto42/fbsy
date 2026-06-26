//! Data structures for `config.json`.
//!
//! Field names use `camelCase` to stay compatible with the existing Python
//! bridge configuration files.

use serde::{Deserialize, Serialize};

/// Runtime configuration required by the bridge process.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeConfig {
    /// HRMS webhook URL that receives attendance events.
    pub vps_webhook_url: String,
    /// Local HTTP API port.
    pub bridge_port: u16,
    /// Optional HRMS API base URL used for job polling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hrms_base_url: Option<String>,
    /// Optional compatibility token for job polling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hrms_api_token: Option<String>,
    /// Job polling interval in seconds.
    pub job_poll_interval_seconds: u64,
    /// When true, the running bridge auto-installs newer releases.
    pub auto_update: bool,
    /// How often (hours) the bridge checks for a newer release.
    pub update_check_interval_hours: u64,
    /// One or more ZKTeco devices managed by this bridge.
    pub devices: Vec<BridgeDeviceConfig>,
}

/// Runtime configuration for one ZKTeco device.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeDeviceConfig {
    /// LAN IP address of the ZKTeco device.
    pub device_ip: String,
    /// ZKTeco device port, normally `4370`.
    pub device_port: u16,
    /// Device communication password/PIN, if configured.
    pub device_password: i32,
    /// Device connection timeout in seconds.
    pub device_timeout: u64,
    /// Whether the device adapter should force UDP mode.
    pub device_force_udp: bool,
    /// Whether the device adapter should skip ping before connecting.
    pub device_omit_ping: bool,
    /// HRMS device identifier used by the webhook.
    pub device_code: String,
    /// Shared webhook secret. This must never be logged raw.
    pub api_key: String,
    /// HRMS organization identifier.
    pub organization_id: u64,
    /// Background sync interval in seconds.
    pub sync_interval_seconds: u64,
    /// Clear attendance from the device only after successful upload.
    pub clear_attendance_after_sync: bool,
}

/// Safe-to-print version of `BridgeConfig`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactedBridgeConfig {
    pub vps_webhook_url: String,
    pub bridge_port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hrms_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hrms_api_token: Option<String>,
    pub job_poll_interval_seconds: u64,
    pub devices: Vec<RedactedBridgeDeviceConfig>,
}

/// Safe-to-print version of one device config.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactedBridgeDeviceConfig {
    pub device_ip: String,
    pub device_port: u16,
    pub device_timeout: u64,
    pub device_force_udp: bool,
    pub device_omit_ping: bool,
    pub device_code: String,
    pub api_key: String,
    pub organization_id: u64,
    pub sync_interval_seconds: u64,
    pub clear_attendance_after_sync: bool,
}
