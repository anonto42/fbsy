use std::{fs, path::PathBuf};

use fingerbridge::config::{BridgeConfig, BridgeMode, ConfigError};
use serde_json::json;

#[test]
fn config_example_exists() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config.example.json");
    assert!(path.exists());
    let raw = fs::read_to_string(path).expect("read config example");
    assert!(raw.contains("devices"));
}

#[test]
fn legacy_single_device_config_is_wrapped_into_devices_array() {
    let cfg = BridgeConfig::from_json_value(json!({
        "deviceIp": "192.168.1.10",
        "deviceCode": "LEGACY-DEVICE",
        "apiKey": "secret",
        "organizationId": 9,
        "vpsWebhookUrl": "https://example.test/webhook"
    }))
    .expect("legacy config should load");

    assert_eq!(cfg.bridge_port, 7431);
    assert_eq!(cfg.bridge_mode, BridgeMode::Pull);
    assert_eq!(cfg.devices.len(), 1);
    assert_eq!(cfg.devices[0].device_code, "LEGACY-DEVICE");
    assert_eq!(cfg.devices[0].organization_id, 9);
    assert_eq!(cfg.devices[0].device_port, 4370);
    assert_eq!(cfg.devices[0].sync_interval_seconds, 300);
}

#[test]
fn push_only_config_can_load_without_pull_devices() {
    let cfg = BridgeConfig::from_json_value(json!({
        "bridgeMode": "push",
        "vpsWebhookUrl": "https://example.test/webhook",
        "autoStartOnBoot": true,
        "devices": [],
        "senseFace": {
            "enabled": true,
            "apiKey": "senseface-secret",
            "organizationId": 7,
            "devices": [{
                "serialNumber": "SF123",
                "deviceCode": "FACE-GATE-01"
            }]
        }
    }))
    .expect("push-only config should load");

    assert_eq!(cfg.bridge_mode, BridgeMode::Push);
    assert!(cfg.auto_start_on_boot);
    assert!(cfg.devices.is_empty());
    let sense_face = cfg.sense_face.expect("senseFace config");
    assert_eq!(sense_face.port, 8090);
    assert_eq!(sense_face.devices[0].api_key, "senseface-secret");
    assert_eq!(sense_face.devices[0].organization_id, 7);
}

#[test]
fn pull_mode_still_requires_pull_devices() {
    let err = BridgeConfig::from_json_value(json!({
        "bridgeMode": "pull",
        "vpsWebhookUrl": "https://example.test/webhook",
        "devices": []
    }))
    .expect_err("pull mode without devices must fail");

    assert!(matches!(err, ConfigError::Invalid(_)));
    assert!(err
        .to_string()
        .contains("devices must be a non-empty array"));
}

#[test]
fn multi_device_config_applies_defaults_per_device() {
    let cfg = BridgeConfig::from_json_value(json!({
        "vpsWebhookUrl": "https://example.test/webhook",
        "bridgePort": 7431,
        "devices": [
            {
                "deviceIp": "192.168.1.10",
                "deviceCode": "DEVICE-1",
                "apiKey": "secret"
            },
            {
                "deviceIp": "192.168.1.11",
                "deviceCode": "DEVICE-2",
                "apiKey": "secret2"
            }
        ]
    }))
    .expect("multi-device config should load");

    assert_eq!(cfg.devices.len(), 2);
    assert_eq!(cfg.devices[0].device_port, 4370);
    assert_eq!(cfg.devices[1].sync_interval_seconds, 300);
    assert!(!cfg.devices[0].clear_attendance_after_sync);
}

#[test]
fn duplicate_device_codes_are_rejected() {
    let err = BridgeConfig::from_json_value(json!({
        "vpsWebhookUrl": "https://example.test/webhook",
        "devices": [
            {
                "deviceIp": "192.168.1.10",
                "deviceCode": "DEVICE-1",
                "apiKey": "secret"
            },
            {
                "deviceIp": "192.168.1.11",
                "deviceCode": "DEVICE-1",
                "apiKey": "secret2"
            }
        ]
    }))
    .expect_err("duplicate deviceCode must fail");

    assert!(matches!(err, ConfigError::Invalid(_)));
    assert!(err.to_string().contains("duplicate deviceCode"));
}

#[test]
fn bool_strings_are_coerced_like_python_bridge() {
    let cfg = BridgeConfig::from_json_value(json!({
        "deviceIp": "192.168.1.10",
        "deviceCode": "DEVICE-1",
        "apiKey": "secret",
        "vpsWebhookUrl": "https://example.test/webhook",
        "clearAttendanceAfterSync": "true",
        "deviceForceUdp": "no",
        "deviceOmitPing": "1"
    }))
    .expect("bool-like strings should load");

    let device = &cfg.devices[0];
    assert!(device.clear_attendance_after_sync);
    assert!(!device.device_force_udp);
    assert!(device.device_omit_ping);
}

#[test]
fn sync_and_job_poll_intervals_are_clamped_to_five_seconds() {
    let cfg = BridgeConfig::from_json_value(json!({
        "deviceIp": "192.168.1.10",
        "deviceCode": "DEVICE-1",
        "apiKey": "secret",
        "vpsWebhookUrl": "https://example.test/webhook",
        "syncIntervalSeconds": 1,
        "jobPollIntervalSeconds": 1
    }))
    .expect("short intervals should be clamped");

    assert_eq!(cfg.devices[0].sync_interval_seconds, 5);
    assert_eq!(cfg.job_poll_interval_seconds, 5);
}

#[test]
fn redacted_config_hides_all_device_api_keys() {
    let cfg = BridgeConfig::from_json_value(json!({
        "vpsWebhookUrl": "https://example.test/webhook",
        "devices": [
            {
                "deviceIp": "192.168.1.10",
                "deviceCode": "DEVICE-1",
                "apiKey": "secret-one"
            },
            {
                "deviceIp": "192.168.1.11",
                "deviceCode": "DEVICE-2",
                "apiKey": "secret-two"
            }
        ]
    }))
    .expect("multi-device config should load");

    let output = serde_json::to_string(&cfg.redacted()).expect("serialize redacted config");
    assert!(!output.contains("secret-one"));
    assert!(!output.contains("secret-two"));
    assert!(output.contains("***"));
}

#[test]
fn invalid_url_is_rejected() {
    let err = BridgeConfig::from_json_value(json!({
        "deviceIp": "192.168.1.10",
        "deviceCode": "DEVICE-1",
        "apiKey": "secret",
        "vpsWebhookUrl": "ftp://example.test/webhook"
    }))
    .expect_err("non-http webhook URL must fail");

    assert!(matches!(err, ConfigError::Invalid(_)));
}
