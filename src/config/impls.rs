//! `BridgeConfig` behavior.
//!
//! This file answers: "What can a loaded config do?"

use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::support::redaction::redact;

use super::{
    BridgeConfig, BridgeDeviceConfig, ConfigError, RedactedBridgeConfig, RedactedBridgeDeviceConfig,
};

const DEFAULT_DEVICE_PORT: u16 = 4370;
const DEFAULT_DEVICE_PASSWORD: i32 = 0;
const DEFAULT_DEVICE_TIMEOUT: u64 = 15;
const DEFAULT_DEVICE_FORCE_UDP: bool = false;
const DEFAULT_DEVICE_OMIT_PING: bool = true;
const DEFAULT_ORGANIZATION_ID: u64 = 1;
const DEFAULT_SYNC_INTERVAL_SECONDS: u64 = 300;
const DEFAULT_CLEAR_ATTENDANCE_AFTER_SYNC: bool = false;
const DEFAULT_BRIDGE_PORT: u16 = 7431;
const DEFAULT_JOB_POLL_INTERVAL_SECONDS: u64 = 30;
const MIN_INTERVAL_SECONDS: u64 = 5;

impl BridgeConfig {
    /// Normalize legacy/new config JSON into the typed runtime model.
    pub fn from_json_value(raw: Value) -> Result<Self, ConfigError> {
        let root = raw
            .as_object()
            .ok_or_else(|| ConfigError::Invalid("config must be a JSON object".into()))?;

        let vps_webhook_url = required_string(root, "vpsWebhookUrl", "config")?;
        let bridge_port = u16_from_value(
            root.get("bridgePort").or_else(|| root.get("port")),
            DEFAULT_BRIDGE_PORT,
            "bridgePort",
            "config",
        )?;
        let hrms_base_url = optional_trimmed_string(root, "hrmsBaseUrl").map(strip_trailing_slash);
        let hrms_api_token = optional_trimmed_string(root, "hrmsApiToken");
        let job_poll_interval_seconds = u64_from_value(
            root.get("jobPollIntervalSeconds"),
            DEFAULT_JOB_POLL_INTERVAL_SECONDS,
            "jobPollIntervalSeconds",
            "config",
        )?
        .max(MIN_INTERVAL_SECONDS);

        let devices = match root.get("devices") {
            Some(Value::Array(items)) if !items.is_empty() => items
                .iter()
                .enumerate()
                .map(|(index, value)| parse_device(value, index))
                .collect::<Result<Vec<_>, _>>()?,
            Some(Value::Array(_)) => {
                return Err(ConfigError::Invalid(
                    "devices must be a non-empty array".into(),
                ));
            }
            Some(_) => {
                return Err(ConfigError::Invalid(
                    "devices must be a non-empty array".into(),
                ));
            }
            None => vec![parse_legacy_device(root)?],
        };

        let cfg = Self {
            vps_webhook_url,
            bridge_port,
            hrms_base_url,
            hrms_api_token,
            job_poll_interval_seconds,
            devices,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    /// Create a support/debug view that hides secrets.
    pub fn redacted(&self) -> RedactedBridgeConfig {
        RedactedBridgeConfig {
            vps_webhook_url: self.vps_webhook_url.clone(),
            bridge_port: self.bridge_port,
            hrms_base_url: self.hrms_base_url.clone(),
            hrms_api_token: self.hrms_api_token.as_deref().map(redact),
            job_poll_interval_seconds: self.job_poll_interval_seconds,
            devices: self
                .devices
                .iter()
                .map(BridgeDeviceConfig::redacted)
                .collect(),
        }
    }

    /// Validate config values after deserialization and default application.
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_http_url(&self.vps_webhook_url, "vpsWebhookUrl")?;
        if let Some(url) = &self.hrms_base_url {
            validate_http_url(url, "hrmsBaseUrl")?;
        }
        if self.devices.is_empty() {
            return Err(ConfigError::Invalid(
                "devices must be a non-empty array".into(),
            ));
        }

        let mut seen_codes = HashSet::new();
        for (index, device) in self.devices.iter().enumerate() {
            device.validate(index)?;
            if !seen_codes.insert(device.device_code.clone()) {
                return Err(ConfigError::Invalid(format!(
                    "duplicate deviceCode '{}' in devices array",
                    device.device_code
                )));
            }
        }
        Ok(())
    }
}

impl BridgeDeviceConfig {
    /// Create a support/debug view that hides device-level secrets.
    pub fn redacted(&self) -> RedactedBridgeDeviceConfig {
        RedactedBridgeDeviceConfig {
            device_ip: self.device_ip.clone(),
            device_port: self.device_port,
            device_timeout: self.device_timeout,
            device_force_udp: self.device_force_udp,
            device_omit_ping: self.device_omit_ping,
            device_code: redact(&self.device_code),
            api_key: redact(&self.api_key),
            organization_id: self.organization_id,
            sync_interval_seconds: self.sync_interval_seconds,
            clear_attendance_after_sync: self.clear_attendance_after_sync,
        }
    }

    /// Validate one device entry after defaults have been applied.
    pub fn validate(&self, index: usize) -> Result<(), ConfigError> {
        let prefix = format!("devices[{index}]: ");
        if self.device_ip.trim().is_empty() {
            return Err(ConfigError::Invalid(format!(
                "{prefix}deviceIp is required"
            )));
        }
        if self.device_code.trim().is_empty() {
            return Err(ConfigError::Invalid(format!(
                "{prefix}deviceCode is required"
            )));
        }
        if self.api_key.trim().is_empty() {
            return Err(ConfigError::Invalid(format!("{prefix}apiKey is required")));
        }
        if !(1..=120).contains(&self.device_timeout) {
            return Err(ConfigError::Invalid(format!(
                "{prefix}deviceTimeout must be between 1 and 120"
            )));
        }
        if self.sync_interval_seconds < MIN_INTERVAL_SECONDS {
            return Err(ConfigError::Invalid(format!(
                "{prefix}syncIntervalSeconds must be at least 5"
            )));
        }
        Ok(())
    }
}

fn parse_legacy_device(root: &Map<String, Value>) -> Result<BridgeDeviceConfig, ConfigError> {
    parse_device_from_object(root, "device")
}

fn parse_device(value: &Value, index: usize) -> Result<BridgeDeviceConfig, ConfigError> {
    let object = value
        .as_object()
        .ok_or_else(|| ConfigError::Invalid(format!("devices[{index}]: must be a JSON object")))?;
    parse_device_from_object(object, &format!("devices[{index}]"))
}

fn parse_device_from_object(
    object: &Map<String, Value>,
    context: &str,
) -> Result<BridgeDeviceConfig, ConfigError> {
    Ok(BridgeDeviceConfig {
        device_ip: required_string(object, "deviceIp", context)?,
        device_port: u16_from_value(
            object.get("devicePort"),
            DEFAULT_DEVICE_PORT,
            "devicePort",
            context,
        )?,
        device_password: i32_from_value(
            object.get("devicePassword"),
            DEFAULT_DEVICE_PASSWORD,
            "devicePassword",
            context,
        )?,
        device_timeout: u64_from_value(
            object.get("deviceTimeout"),
            DEFAULT_DEVICE_TIMEOUT,
            "deviceTimeout",
            context,
        )?,
        device_force_udp: bool_from_value(
            object.get("deviceForceUdp"),
            DEFAULT_DEVICE_FORCE_UDP,
            "deviceForceUdp",
            context,
        )?,
        device_omit_ping: bool_from_value(
            object.get("deviceOmitPing"),
            DEFAULT_DEVICE_OMIT_PING,
            "deviceOmitPing",
            context,
        )?,
        device_code: required_string(object, "deviceCode", context)?,
        api_key: required_string(object, "apiKey", context)?,
        organization_id: u64_from_value(
            object.get("organizationId"),
            DEFAULT_ORGANIZATION_ID,
            "organizationId",
            context,
        )?,
        sync_interval_seconds: u64_from_value(
            object.get("syncIntervalSeconds"),
            DEFAULT_SYNC_INTERVAL_SECONDS,
            "syncIntervalSeconds",
            context,
        )?
        .max(MIN_INTERVAL_SECONDS),
        clear_attendance_after_sync: bool_from_value(
            object.get("clearAttendanceAfterSync"),
            DEFAULT_CLEAR_ATTENDANCE_AFTER_SYNC,
            "clearAttendanceAfterSync",
            context,
        )?,
    })
}

fn required_string(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<String, ConfigError> {
    optional_trimmed_string(object, key)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ConfigError::Invalid(format!("{context}: missing required key: {key}")))
}

fn optional_trimmed_string(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(|value| match value {
        Value::String(text) => Some(text.trim().to_string()),
        Value::Null => None,
        other => Some(other.to_string().trim_matches('"').trim().to_string()),
    })
}

fn strip_trailing_slash(value: String) -> String {
    value.trim_end_matches('/').to_string()
}

fn u16_from_value(
    value: Option<&Value>,
    default: u16,
    key: &str,
    context: &str,
) -> Result<u16, ConfigError> {
    let number = u64_from_value(value, default.into(), key, context)?;
    u16::try_from(number)
        .ok()
        .filter(|port| *port >= 1)
        .ok_or_else(|| ConfigError::Invalid(format!("{context}: {key} must be 1-65535")))
}

fn u64_from_value(
    value: Option<&Value>,
    default: u64,
    key: &str,
    context: &str,
) -> Result<u64, ConfigError> {
    match value {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| ConfigError::Invalid(format!("{context}: {key} must be an integer"))),
        Some(Value::String(text)) => text
            .trim()
            .parse::<u64>()
            .map_err(|_| ConfigError::Invalid(format!("{context}: {key} must be an integer"))),
        Some(_) => Err(ConfigError::Invalid(format!(
            "{context}: {key} must be an integer"
        ))),
    }
}

fn i32_from_value(
    value: Option<&Value>,
    default: i32,
    key: &str,
    context: &str,
) -> Result<i32, ConfigError> {
    match value {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Number(number)) => number
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| ConfigError::Invalid(format!("{context}: {key} must be an integer"))),
        Some(Value::String(text)) => text
            .trim()
            .parse::<i32>()
            .map_err(|_| ConfigError::Invalid(format!("{context}: {key} must be an integer"))),
        Some(_) => Err(ConfigError::Invalid(format!(
            "{context}: {key} must be an integer"
        ))),
    }
}

fn bool_from_value(
    value: Option<&Value>,
    default: bool,
    key: &str,
    context: &str,
) -> Result<bool, ConfigError> {
    match value {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(Value::Number(number)) => Ok(number.as_i64().unwrap_or_default() != 0),
        Some(Value::String(text)) => {
            let normalized = text.trim().to_ascii_lowercase();
            Ok(matches!(
                normalized.as_str(),
                "1" | "true" | "yes" | "y" | "on"
            ))
        }
        Some(_) => Err(ConfigError::Invalid(format!(
            "{context}: {key} must be a boolean"
        ))),
    }
}

fn validate_http_url(value: &str, key: &str) -> Result<(), ConfigError> {
    let has_scheme = value.starts_with("http://") || value.starts_with("https://");
    let has_host = value
        .split_once("://")
        .map(|(_, rest)| !rest.trim_matches('/').is_empty())
        .unwrap_or(false);
    if has_scheme && has_host {
        Ok(())
    } else {
        Err(ConfigError::Invalid(format!(
            "{key} must use http:// or https://"
        )))
    }
}
