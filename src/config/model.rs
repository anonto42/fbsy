//! Data structures for `config.json`.
//!
//! Field names use `camelCase` to stay compatible with the existing Python
//! bridge configuration files.

use serde::{Deserialize, Serialize};

/// How the bridge receives attendance.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BridgeMode {
    /// Pull attendance from classic ZKTeco devices over TCP 4370.
    Pull,
    /// Receive attendance pushed by SenseFace / ADMS terminals over HTTP.
    Push,
    /// Run both pull schedulers and the push receiver in one bridge process.
    Hybrid,
}

/// Runtime configuration required by the bridge process.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeConfig {
    /// Whether this bridge works in pull, push, or hybrid mode.
    pub bridge_mode: BridgeMode,
    /// HRMS webhook URL that receives attendance events.
    pub vps_webhook_url: String,
    /// Local HTTP API port.
    pub bridge_port: u16,
    /// Whether the bridge should attempt to register itself for OS boot.
    pub auto_start_on_boot: bool,
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
    /// Optional SenseFace / ADMS push receiver configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sense_face: Option<SenseFaceConfig>,
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
    /// Timezone the device's naive timestamps are in (e.g. `+06:00` or `Asia/Dhaka`).
    /// `None` means UTC, preserving the legacy Python-bridge behavior.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_timezone: Option<String>,
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
    /// Clear device attendance when pulled record count reaches this threshold.
    /// Ignored when `clear_attendance_after_sync` is true (already always clears).
    /// Use this for old firmware that can't be cleared every sync but needs
    /// periodic clearing before device memory wraps (e.g. set to 1000 for F22).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clear_attendance_threshold: Option<u64>,
}

/// Safe-to-print version of `BridgeConfig`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactedBridgeConfig {
    pub bridge_mode: BridgeMode,
    pub vps_webhook_url: String,
    pub bridge_port: u16,
    pub auto_start_on_boot: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hrms_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hrms_api_token: Option<String>,
    pub job_poll_interval_seconds: u64,
    pub devices: Vec<RedactedBridgeDeviceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sense_face: Option<RedactedSenseFaceConfig>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_timezone: Option<String>,
    pub device_code: String,
    pub api_key: String,
    pub organization_id: u64,
    pub sync_interval_seconds: u64,
    pub clear_attendance_after_sync: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clear_attendance_threshold: Option<u64>,
}

/// SenseFace / ADMS push receiver configuration.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SenseFaceConfig {
    /// Start the HTTP receiver inside the bridge service.
    pub enabled: bool,
    /// Bind host for devices to connect to. Usually `0.0.0.0`.
    pub bind_host: String,
    /// HTTP port used by the terminal cloud server settings.
    pub port: u16,
    /// Device wall-clock timezone label or fixed offset.
    pub timezone: String,
    /// Default HRMS device code prefix when a serial has no explicit mapping.
    pub device_code_prefix: String,
    /// Default HRMS API key for pushed devices.
    pub api_key: String,
    /// Default HRMS organization id for pushed devices.
    pub organization_id: u64,
    /// How often pending pushed attendance should be forwarded to HRMS.
    pub forward_interval_seconds: u64,
    /// Explicit serial-number to HRMS device mappings.
    pub devices: Vec<SenseFaceDeviceConfig>,
}

/// Mapping for one SenseFace terminal serial number.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SenseFaceDeviceConfig {
    pub serial_number: String,
    pub device_code: String,
    pub api_key: String,
    pub organization_id: u64,
}

/// Safe-to-print version of SenseFace config.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactedSenseFaceConfig {
    pub enabled: bool,
    pub bind_host: String,
    pub port: u16,
    pub timezone: String,
    pub device_code_prefix: String,
    pub api_key: String,
    pub organization_id: u64,
    pub forward_interval_seconds: u64,
    pub devices: Vec<RedactedSenseFaceDeviceConfig>,
}

/// Safe-to-print version of one SenseFace device mapping.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactedSenseFaceDeviceConfig {
    pub serial_number: String,
    pub device_code: String,
    pub api_key: String,
    pub organization_id: u64,
}
