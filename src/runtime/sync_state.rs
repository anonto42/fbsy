//! Per-device sync state and sync lifecycle.
//!
//! Safety rule: never clear device attendance unless HRMS upload succeeded.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use crate::{
    config::BridgeDeviceConfig,
    domain::{to_hrms_events, SyncResult},
    ports::{
        device::{DeviceClient, DeviceConnector},
        hrms::HrmsClient,
    },
    support::log::{self, Level},
};

/// Runtime state for one configured device.
pub struct DeviceSyncState {
    device: BridgeDeviceConfig,
    webhook_url: String,
    connector: Arc<dyn DeviceConnector>,
    hrms: Arc<dyn HrmsClient>,
    log_progress: bool,
    lock: Mutex<()>,
    last_result: Mutex<Option<SyncResult>>,
}

impl DeviceSyncState {
    pub fn new(
        device: BridgeDeviceConfig,
        webhook_url: String,
        connector: Arc<dyn DeviceConnector>,
        hrms: Arc<dyn HrmsClient>,
    ) -> Self {
        Self {
            device,
            webhook_url,
            connector,
            hrms,
            log_progress: false,
            lock: Mutex::new(()),
            last_result: Mutex::new(None),
        }
    }

    /// Enable progress logs for long-running service mode.
    pub fn with_progress_logging(mut self) -> Self {
        self.log_progress = true;
        self
    }

    pub fn device_code(&self) -> &str {
        &self.device.device_code
    }

    pub fn sync_interval_seconds(&self) -> u64 {
        self.device.sync_interval_seconds
    }

    pub fn syncing(&self) -> bool {
        self.lock.try_lock().is_err()
    }

    pub fn last_result(&self) -> Option<SyncResult> {
        self.last_result
            .lock()
            .ok()
            .and_then(|result| result.clone())
    }

    /// Run one sync attempt for this device.
    pub fn sync_once(&self) -> SyncResult {
        let started_at = now_iso();
        self.log(Level::Info, format_args!("sync start at {started_at}"));

        let guard = match self.lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                self.log(
                    Level::Warn,
                    format_args!("sync skipped: already in progress"),
                );
                return self.store_result(SyncResult {
                    ok: false,
                    device_code: self.device.device_code.clone(),
                    pulled: 0,
                    forwarded: 0,
                    device_attendance_cleared: false,
                    started_at,
                    message: "sync already in progress".to_string(),
                });
            }
        };

        let result = self.sync_once_locked(started_at);
        drop(guard);
        let level = if result.ok { Level::Info } else { Level::Error };
        self.log(
            level,
            format_args!(
                "sync done: ok={} pulled={} forwarded={} cleared={} message=\"{}\"",
                result.ok,
                result.pulled,
                result.forwarded,
                result.device_attendance_cleared,
                result.message
            ),
        );
        self.store_result(result)
    }

    fn sync_once_locked(&self, started_at: String) -> SyncResult {
        self.log(
            Level::Info,
            format_args!(
                "calling device {}:{} (timeout={}s udp={} omit_ping={})",
                self.device.device_ip,
                self.device.device_port,
                self.device.device_timeout,
                self.device.device_force_udp,
                self.device.device_omit_ping
            ),
        );
        let mut client = match self.connector.connect(&self.device) {
            Ok(client) => client,
            Err(err) => {
                let message = sanitize(&err.to_string(), &self.device);
                self.log(Level::Error, format_args!("device call failed: {message}"));
                return self.failure(0, 0, started_at, message);
            }
        };
        self.log(Level::Info, format_args!("device responded: connected"));

        let result = self.sync_with_client(client.as_mut(), started_at);
        client.disconnect();
        self.log(Level::Info, format_args!("device disconnected"));
        result
    }

    fn sync_with_client(&self, client: &mut dyn DeviceClient, started_at: String) -> SyncResult {
        self.log(Level::Info, format_args!("reading attendance from device"));
        let attendance = match client.pull_attendance() {
            Ok(records) => {
                self.log(
                    Level::Info,
                    format_args!("device returned {} attendance record(s)", records.len()),
                );
                records
            }
            Err(err) => {
                let message = sanitize(&err.to_string(), &self.device);
                self.log(Level::Error, format_args!("device read failed: {message}"));
                return self.failure(0, 0, started_at, message);
            }
        };

        let events = to_hrms_events(&attendance);
        self.log(
            Level::Info,
            format_args!(
                "mapped {} record(s) into {} HRMS event(s)",
                attendance.len(),
                events.len()
            ),
        );
        if events.is_empty() {
            self.log(Level::Info, format_args!("no HRMS events to forward"));
            return SyncResult {
                ok: true,
                device_code: self.device.device_code.clone(),
                pulled: attendance.len(),
                forwarded: 0,
                device_attendance_cleared: false,
                started_at,
                message: "no events to forward".to_string(),
            };
        }

        self.log(
            Level::Info,
            format_args!("forwarding {} event(s) to HRMS webhook", events.len()),
        );
        let webhook = match self
            .hrms
            .forward_events(&self.webhook_url, &self.device, &events)
        {
            Ok(result) => {
                self.log(
                    Level::Info,
                    format_args!(
                        "forwarded {}/{} event(s) to HRMS → ok",
                        result.received,
                        events.len()
                    ),
                );
                result
            }
            Err(err) => {
                let message = sanitize(&err.to_string(), &self.device);
                self.log(Level::Error, format_args!("HRMS forward failed: {message}"));
                return self.failure(attendance.len(), 0, started_at, message);
            }
        };

        let mut cleared = false;
        let mut message = format!("forwarded {} event(s)", events.len());
        if self.device.clear_attendance_after_sync {
            self.log(Level::Info, format_args!("clearing attendance on device"));
            match client.clear_attendance() {
                Ok(()) => {
                    cleared = true;
                    self.log(Level::Info, format_args!("device attendance cleared"));
                }
                Err(err) => {
                    let safe_error = sanitize(&err.to_string(), &self.device);
                    self.log(
                        Level::Warn,
                        format_args!("clear attendance failed: {safe_error}"),
                    );
                    message =
                        format!("forwarded events but failed to clear attendance: {safe_error}");
                }
            }
        } else {
            self.log(
                Level::Info,
                format_args!("clear attendance disabled; records remain on device"),
            );
        }

        SyncResult {
            ok: true,
            device_code: self.device.device_code.clone(),
            pulled: attendance.len(),
            forwarded: webhook.received,
            device_attendance_cleared: cleared,
            started_at,
            message,
        }
    }

    fn failure(
        &self,
        pulled: usize,
        forwarded: usize,
        started_at: String,
        message: String,
    ) -> SyncResult {
        SyncResult {
            ok: false,
            device_code: self.device.device_code.clone(),
            pulled,
            forwarded,
            device_attendance_cleared: false,
            started_at,
            message,
        }
    }

    fn store_result(&self, result: SyncResult) -> SyncResult {
        if let Ok(mut last) = self.last_result.lock() {
            *last = Some(result.clone());
        }
        result
    }

    fn log(&self, level: Level, args: std::fmt::Arguments<'_>) {
        if self.log_progress {
            log::event(level, &format!("sync {}", self.device.device_code), args);
        }
    }
}

fn now_iso() -> String {
    let now: DateTime<Utc> = Utc::now();
    now.to_rfc3339()
}

fn sanitize(message: &str, device: &BridgeDeviceConfig) -> String {
    let mut s = message.replace(&device.api_key, "***");
    s = s.replace(&device.device_code, "***");
    if s.len() > 200 {
        s.truncate(200);
        s.push_str("...");
    }
    s
}
