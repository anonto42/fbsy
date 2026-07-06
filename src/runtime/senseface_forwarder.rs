use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use anyhow::Result;

use crate::{
    config::{BridgeDeviceConfig, SenseFaceConfig},
    domain::senseface::{senseface_timezone_offset, to_hrms_events_from_senseface},
    ports::{hrms::HrmsClient, senseface_store::SenseFaceStore},
    support::log,
};

pub fn start_senseface_forwarder(
    store: Arc<dyn SenseFaceStore>,
    config: SenseFaceConfig,
    webhook_url: String,
    hrms: Arc<dyn HrmsClient>,
    shutdown: Arc<AtomicBool>,
) {
    let interval = config.forward_interval_seconds.max(5);
    let sf_config = config.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(10));
        log::info(
            "sf-fwd",
            format_args!("SenseFace forwarder started (interval={interval}s, batch=200)"),
        );
        loop {
            for _ in 0..interval {
                if shutdown.load(Ordering::SeqCst) {
                    log::info("sf-fwd", format_args!("forwarder stopping"));
                    return;
                }
                thread::sleep(Duration::from_secs(1));
            }
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            if let Err(err) = forward_once(&store, &sf_config, &webhook_url, &*hrms) {
                log::warn("sf-fwd", format_args!("forward cycle failed: {err}"));
            }
        }
    });
}

fn forward_once(
    store: &Arc<dyn SenseFaceStore>,
    config: &SenseFaceConfig,
    webhook_url: &str,
    hrms: &dyn HrmsClient,
) -> Result<()> {
    let pending = store.get_pending_forward_attendance(200)?;
    if pending.is_empty() {
        return Ok(());
    }

    let mut by_serial: std::collections::BTreeMap<String, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, record) in pending.iter().enumerate() {
        by_serial
            .entry(record.serial_number.clone())
            .or_default()
            .push(i);
    }

    let offset = senseface_timezone_offset(&config.timezone)
        .unwrap_or_else(crate::domain::default_utc_offset);

    for (serial, indices) in &by_serial {
        let device = resolve_device_config(serial, config);
        let records: Vec<_> = indices.iter().map(|i| &pending[*i]).cloned().collect();
        let ids: Vec<i64> = records.iter().map(|r| r.id).collect();

        let events = to_hrms_events_from_senseface(
            &records,
            offset,
            &device.device_code,
            &device.api_key,
            device.organization_id,
        );

        if events.is_empty() {
            store.mark_attendance_forwarded(&ids)?;
            continue;
        }

        match hrms.forward_events(webhook_url, &device, &events) {
            Ok(result) => {
                log::info(
                    "sf-fwd",
                    format_args!(
                        "forwarded {}/{} SenseFace events for {} (serial={})",
                        result.received,
                        events.len(),
                        device.device_code,
                        serial,
                    ),
                );
                store.mark_attendance_forwarded(&ids)?;
            }
            Err(err) => {
                log::warn(
                    "sf-fwd",
                    format_args!(
                        "HRMS forward failed for {} (serial={}): {err}",
                        device.device_code, serial,
                    ),
                );
            }
        }
    }

    Ok(())
}

fn resolve_device_config(serial: &str, config: &SenseFaceConfig) -> BridgeDeviceConfig {
    if let Some(mapping) = config.devices.iter().find(|d| d.serial_number == serial) {
        return BridgeDeviceConfig {
            device_ip: String::new(),
            device_port: 0,
            device_password: 0,
            device_timeout: 15,
            device_force_udp: false,
            device_omit_ping: true,
            device_timezone: None,
            device_code: mapping.device_code.clone(),
            api_key: mapping.api_key.clone(),
            organization_id: mapping.organization_id,
            sync_interval_seconds: 300,
            clear_attendance_after_sync: false,
            clear_attendance_threshold: None,
        };
    }

    BridgeDeviceConfig {
        device_ip: String::new(),
        device_port: 0,
        device_password: 0,
        device_timeout: 15,
        device_force_udp: false,
        device_omit_ping: true,
        device_timezone: None,
        device_code: format!("{}-{}", config.device_code_prefix, serial),
        api_key: config.api_key.clone(),
        organization_id: config.organization_id,
        sync_interval_seconds: 300,
        clear_attendance_after_sync: false,
            clear_attendance_threshold: None,
    }
}
