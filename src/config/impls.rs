//! `BridgeConfig` behavior.
//!
//! This file answers: "What can a loaded config do?"

use std::collections::HashSet;

use chrono::FixedOffset;
use serde_json::{Map, Value};

use crate::domain::{default_utc_offset, parse_utc_offset};
use crate::support::redaction::redact;

use super::{
    BridgeConfig, BridgeDeviceConfig, BridgeMode, ConfigError, RedactedBridgeConfig,
    RedactedBridgeDeviceConfig, RedactedSenseFaceConfig, RedactedSenseFaceDeviceConfig,
    SenseFaceConfig, SenseFaceDeviceConfig,
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
const DEFAULT_SENSEFACE_PORT: u16 = 8090;
const DEFAULT_SENSEFACE_BIND_HOST: &str = "0.0.0.0";
const DEFAULT_SENSEFACE_TIMEZONE: &str = "Asia/Dhaka";
const DEFAULT_SENSEFACE_DEVICE_CODE_PREFIX: &str = "SF";
const DEFAULT_SENSEFACE_FORWARD_INTERVAL_SECONDS: u64 = 10;
const MIN_INTERVAL_SECONDS: u64 = 5;

impl BridgeConfig {
    /// Normalize legacy/new config JSON into the typed runtime model.
    pub fn from_json_value(raw: Value) -> Result<Self, ConfigError> {
        let root = raw
            .as_object()
            .ok_or_else(|| ConfigError::Invalid("config must be a JSON object".into()))?;

        let vps_webhook_url = required_string(root, "vpsWebhookUrl", "config")?;
        let sense_face = parse_sense_face(root.get("senseFace"))?;
        let bridge_mode = parse_bridge_mode(root.get("bridgeMode"), sense_face.as_ref())?;
        let bridge_port = u16_from_value(
            root.get("bridgePort").or_else(|| root.get("port")),
            DEFAULT_BRIDGE_PORT,
            "bridgePort",
            "config",
        )?;
        let auto_start_on_boot = bool_from_value(
            root.get("autoStartOnBoot"),
            false,
            "autoStartOnBoot",
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

        let auto_update = bool_from_value(root.get("autoUpdate"), false, "autoUpdate", "config")?;
        let update_check_interval_hours = u64_from_value(
            root.get("updateCheckIntervalHours"),
            6,
            "updateCheckIntervalHours",
            "config",
        )?
        .max(1);

        let devices = match root.get("devices") {
            Some(Value::Array(items)) if !items.is_empty() => items
                .iter()
                .enumerate()
                .map(|(index, value)| parse_device(value, index))
                .collect::<Result<Vec<_>, _>>()?,
            Some(Value::Array(_)) => Vec::new(),
            Some(_) => {
                return Err(ConfigError::Invalid("devices must be an array".into()));
            }
            None if has_legacy_device(root) => vec![parse_legacy_device(root)?],
            None => Vec::new(),
        };

        let cfg = Self {
            bridge_mode,
            vps_webhook_url,
            bridge_port,
            auto_start_on_boot,
            hrms_base_url,
            hrms_api_token,
            job_poll_interval_seconds,
            auto_update,
            update_check_interval_hours,
            devices,
            sense_face,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    /// Create a support/debug view that hides secrets.
    pub fn redacted(&self) -> RedactedBridgeConfig {
        RedactedBridgeConfig {
            bridge_mode: self.bridge_mode,
            vps_webhook_url: self.vps_webhook_url.clone(),
            bridge_port: self.bridge_port,
            auto_start_on_boot: self.auto_start_on_boot,
            hrms_base_url: self.hrms_base_url.clone(),
            hrms_api_token: self.hrms_api_token.as_deref().map(redact),
            job_poll_interval_seconds: self.job_poll_interval_seconds,
            devices: self
                .devices
                .iter()
                .map(BridgeDeviceConfig::redacted)
                .collect(),
            sense_face: self.sense_face.as_ref().map(SenseFaceConfig::redacted),
        }
    }

    /// Validate config values after deserialization and default application.
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_http_url(&self.vps_webhook_url, "vpsWebhookUrl")?;
        if let Some(url) = &self.hrms_base_url {
            validate_http_url(url, "hrmsBaseUrl")?;
        }
        if self.uses_pull() && self.devices.is_empty() {
            return Err(ConfigError::Invalid(
                "devices must be a non-empty array when bridgeMode is pull or hybrid".into(),
            ));
        }
        if self.uses_push() {
            let Some(sense_face) = &self.sense_face else {
                return Err(ConfigError::Invalid(
                    "senseFace config is required when bridgeMode is push or hybrid".into(),
                ));
            };
            sense_face.validate()?;
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

    /// True when classic pull-device sync should run.
    pub fn uses_pull(&self) -> bool {
        matches!(self.bridge_mode, BridgeMode::Pull | BridgeMode::Hybrid)
    }

    /// True when the SenseFace / ADMS push receiver should run.
    pub fn uses_push(&self) -> bool {
        matches!(self.bridge_mode, BridgeMode::Push | BridgeMode::Hybrid)
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
            device_timezone: self.device_timezone.clone(),
            device_code: redact(&self.device_code),
            api_key: redact(&self.api_key),
            organization_id: self.organization_id,
            sync_interval_seconds: self.sync_interval_seconds,
            clear_attendance_after_sync: self.clear_attendance_after_sync,
            clear_attendance_threshold: self.clear_attendance_threshold,
        }
    }

    /// Fixed UTC offset that this device's naive timestamps are in.
    ///
    /// Falls back to UTC when no (or an unparseable) `deviceTimezone` is set;
    /// `validate` rejects unparseable values before runtime ever sees them.
    pub fn utc_offset(&self) -> FixedOffset {
        self.device_timezone
            .as_deref()
            .and_then(parse_utc_offset)
            .unwrap_or_else(default_utc_offset)
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
        if let Some(timezone) = self.device_timezone.as_deref() {
            if parse_utc_offset(timezone).is_none() {
                return Err(ConfigError::Invalid(format!(
                    "{prefix}deviceTimezone must be UTC or a fixed offset like +06:00 (got '{timezone}')"
                )));
            }
        }
        Ok(())
    }
}

impl SenseFaceConfig {
    /// Safe-to-print SenseFace config.
    pub fn redacted(&self) -> RedactedSenseFaceConfig {
        RedactedSenseFaceConfig {
            enabled: self.enabled,
            bind_host: self.bind_host.clone(),
            port: self.port,
            timezone: self.timezone.clone(),
            device_code_prefix: self.device_code_prefix.clone(),
            api_key: redact(&self.api_key),
            organization_id: self.organization_id,
            forward_interval_seconds: self.forward_interval_seconds,
            devices: self
                .devices
                .iter()
                .map(SenseFaceDeviceConfig::redacted)
                .collect(),
        }
    }

    /// Validate SenseFace push receiver settings.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !self.enabled {
            return Err(ConfigError::Invalid(
                "senseFace.enabled must be true when bridgeMode uses push".into(),
            ));
        }
        if self.bind_host.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "senseFace.bindHost is required".into(),
            ));
        }
        if self.timezone.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "senseFace.timezone is required".into(),
            ));
        }
        if self.device_code_prefix.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "senseFace.deviceCodePrefix is required".into(),
            ));
        }
        if self.api_key.trim().is_empty() {
            return Err(ConfigError::Invalid("senseFace.apiKey is required".into()));
        }
        if self.forward_interval_seconds < MIN_INTERVAL_SECONDS {
            return Err(ConfigError::Invalid(
                "senseFace.forwardIntervalSeconds must be at least 5".into(),
            ));
        }

        let mut seen_serials = HashSet::new();
        let mut seen_codes = HashSet::new();
        for (index, device) in self.devices.iter().enumerate() {
            device.validate(index)?;
            if !seen_serials.insert(device.serial_number.clone()) {
                return Err(ConfigError::Invalid(format!(
                    "duplicate serialNumber '{}' in senseFace.devices array",
                    device.serial_number
                )));
            }
            if !seen_codes.insert(device.device_code.clone()) {
                return Err(ConfigError::Invalid(format!(
                    "duplicate deviceCode '{}' in senseFace.devices array",
                    device.device_code
                )));
            }
        }
        Ok(())
    }
}

impl SenseFaceDeviceConfig {
    /// Safe-to-print SenseFace device mapping.
    pub fn redacted(&self) -> RedactedSenseFaceDeviceConfig {
        RedactedSenseFaceDeviceConfig {
            serial_number: redact(&self.serial_number),
            device_code: redact(&self.device_code),
            api_key: redact(&self.api_key),
            organization_id: self.organization_id,
        }
    }

    /// Validate one SenseFace serial mapping.
    pub fn validate(&self, index: usize) -> Result<(), ConfigError> {
        let prefix = format!("senseFace.devices[{index}]: ");
        if self.serial_number.trim().is_empty() {
            return Err(ConfigError::Invalid(format!(
                "{prefix}serialNumber is required"
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
        Ok(())
    }
}

fn parse_legacy_device(root: &Map<String, Value>) -> Result<BridgeDeviceConfig, ConfigError> {
    parse_device_from_object(root, "device")
}

fn has_legacy_device(root: &Map<String, Value>) -> bool {
    root.contains_key("deviceIp") || root.contains_key("deviceCode") || root.contains_key("apiKey")
}

fn parse_bridge_mode(
    value: Option<&Value>,
    sense_face: Option<&SenseFaceConfig>,
) -> Result<BridgeMode, ConfigError> {
    let Some(value) = value else {
        return Ok(if sense_face.as_ref().is_some_and(|cfg| cfg.enabled) {
            BridgeMode::Push
        } else {
            BridgeMode::Pull
        });
    };
    let raw = match value {
        Value::String(text) => text.trim().to_ascii_lowercase(),
        other => other
            .to_string()
            .trim_matches('"')
            .trim()
            .to_ascii_lowercase(),
    };
    match raw.as_str() {
        "pull" => Ok(BridgeMode::Pull),
        "push" => Ok(BridgeMode::Push),
        "hybrid" | "both" | "pull_push" | "push_pull" => Ok(BridgeMode::Hybrid),
        _ => Err(ConfigError::Invalid(
            "config: bridgeMode must be pull, push, or hybrid".into(),
        )),
    }
}

fn parse_sense_face(value: Option<&Value>) -> Result<Option<SenseFaceConfig>, ConfigError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let object = value
        .as_object()
        .ok_or_else(|| ConfigError::Invalid("senseFace must be a JSON object".into()))?;
    let enabled = bool_from_value(object.get("enabled"), true, "enabled", "senseFace")?;
    let bind_host = optional_trimmed_string(object, "bindHost")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_SENSEFACE_BIND_HOST.to_string());
    let port = u16_from_value(
        object.get("port"),
        DEFAULT_SENSEFACE_PORT,
        "port",
        "senseFace",
    )?;
    let timezone = optional_trimmed_string(object, "timezone")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_SENSEFACE_TIMEZONE.to_string());
    let device_code_prefix = optional_trimmed_string(object, "deviceCodePrefix")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_SENSEFACE_DEVICE_CODE_PREFIX.to_string());
    let api_key = required_string(object, "apiKey", "senseFace")?;
    let organization_id = u64_from_value(
        object.get("organizationId"),
        DEFAULT_ORGANIZATION_ID,
        "organizationId",
        "senseFace",
    )?;
    let forward_interval_seconds = u64_from_value(
        object.get("forwardIntervalSeconds"),
        DEFAULT_SENSEFACE_FORWARD_INTERVAL_SECONDS,
        "forwardIntervalSeconds",
        "senseFace",
    )?
    .max(MIN_INTERVAL_SECONDS);
    let devices = match object.get("devices") {
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(index, value)| {
                parse_sense_face_device(value, index, api_key.as_str(), organization_id)
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(ConfigError::Invalid(
                "senseFace.devices must be an array".into(),
            ));
        }
        None => Vec::new(),
    };
    Ok(Some(SenseFaceConfig {
        enabled,
        bind_host,
        port,
        timezone,
        device_code_prefix,
        api_key,
        organization_id,
        forward_interval_seconds,
        devices,
    }))
}

fn parse_sense_face_device(
    value: &Value,
    index: usize,
    default_api_key: &str,
    default_organization_id: u64,
) -> Result<SenseFaceDeviceConfig, ConfigError> {
    let object = value.as_object().ok_or_else(|| {
        ConfigError::Invalid(format!("senseFace.devices[{index}]: must be a JSON object"))
    })?;
    Ok(SenseFaceDeviceConfig {
        serial_number: required_string(
            object,
            "serialNumber",
            &format!("senseFace.devices[{index}]"),
        )?,
        device_code: required_string(object, "deviceCode", &format!("senseFace.devices[{index}]"))?,
        api_key: optional_trimmed_string(object, "apiKey")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default_api_key.to_string()),
        organization_id: u64_from_value(
            object.get("organizationId"),
            default_organization_id,
            "organizationId",
            &format!("senseFace.devices[{index}]"),
        )?,
    })
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
        device_timezone: optional_trimmed_string(object, "deviceTimezone")
            .filter(|value| !value.is_empty()),
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
        clear_attendance_threshold: match object.get("clearAttendanceThreshold") {
            None | Some(Value::Null) => None,
            Some(v) => Some(u64_from_value(
                Some(v),
                0,
                "clearAttendanceThreshold",
                context,
            )?),
        },
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
