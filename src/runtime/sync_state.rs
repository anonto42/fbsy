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
};

/// Runtime state for one configured device.
pub struct DeviceSyncState {
    device: BridgeDeviceConfig,
    webhook_url: String,
    connector: Arc<dyn DeviceConnector>,
    hrms: Arc<dyn HrmsClient>,
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
            lock: Mutex::new(()),
            last_result: Mutex::new(None),
        }
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

        let guard = match self.lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
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
        self.store_result(result)
    }

    fn sync_once_locked(&self, started_at: String) -> SyncResult {
        let mut client = match self.connector.connect(&self.device) {
            Ok(client) => client,
            Err(err) => {
                return self.failure(0, 0, started_at, sanitize(&err.to_string(), &self.device));
            }
        };

        let result = self.sync_with_client(client.as_mut(), started_at);
        client.disconnect();
        result
    }

    fn sync_with_client(&self, client: &mut dyn DeviceClient, started_at: String) -> SyncResult {
        let attendance = match client.pull_attendance() {
            Ok(records) => records,
            Err(err) => {
                return self.failure(0, 0, started_at, sanitize(&err.to_string(), &self.device));
            }
        };

        let events = to_hrms_events(&attendance);
        if events.is_empty() {
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

        let webhook = match self
            .hrms
            .forward_events(&self.webhook_url, &self.device, &events)
        {
            Ok(result) => result,
            Err(err) => {
                return self.failure(
                    attendance.len(),
                    0,
                    started_at,
                    sanitize(&err.to_string(), &self.device),
                );
            }
        };

        let mut cleared = false;
        let mut message = format!("forwarded {} event(s)", events.len());
        if self.device.clear_attendance_after_sync {
            match client.clear_attendance() {
                Ok(()) => cleared = true,
                Err(err) => {
                    message = format!(
                        "forwarded events but failed to clear attendance: {}",
                        sanitize(&err.to_string(), &self.device)
                    );
                }
            }
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
