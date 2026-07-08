use std::{
    sync::{mpsc, Arc, Condvar, Mutex},
    thread,
};

use fingerbridge::{
    config::BridgeDeviceConfig,
    domain::{DeviceUser, FingerTemplate, HrmsEvent, RawAttendance},
    ports::{
        device::{DeviceClient, DeviceConnector, DeviceError},
        hrms::{HrmsClient, HrmsError, WebhookResult},
    },
    runtime::DeviceSyncState,
};

fn device_config(clear_attendance_after_sync: bool) -> BridgeDeviceConfig {
    BridgeDeviceConfig {
        device_ip: "127.0.0.1".to_string(),
        device_port: 4370,
        device_password: 0,
        device_timeout: 15,
        device_force_udp: false,
        device_omit_ping: true,
        device_timezone: None,
        device_code: "DEVICE-1".to_string(),
        api_key: "secret-key".to_string(),
        organization_id: 1,
        sync_interval_seconds: 300,
        clear_attendance_after_sync,
        clear_attendance_threshold: None,
    }
}

#[derive(Debug, Default)]
struct FakeDeviceInner {
    attendance: Vec<RawAttendance>,
    pull_error: Option<String>,
    clear_error: Option<String>,
    cleared: bool,
    disconnected: bool,
}

#[derive(Clone, Debug)]
struct FakeConnector {
    inner: Arc<Mutex<FakeDeviceInner>>,
}

impl FakeConnector {
    fn with_attendance(attendance: Vec<RawAttendance>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeDeviceInner {
                attendance,
                ..FakeDeviceInner::default()
            })),
        }
    }

    fn cleared(&self) -> bool {
        self.inner.lock().expect("fake lock").cleared
    }

    fn disconnected(&self) -> bool {
        self.inner.lock().expect("fake lock").disconnected
    }

    fn fail_clear(&self, message: &str) {
        self.inner.lock().expect("fake lock").clear_error = Some(message.to_string());
    }
}

impl DeviceConnector for FakeConnector {
    fn connect(&self, _cfg: &BridgeDeviceConfig) -> Result<Box<dyn DeviceClient>, DeviceError> {
        Ok(Box::new(FakeClient {
            inner: Arc::clone(&self.inner),
        }))
    }
}

struct FakeClient {
    inner: Arc<Mutex<FakeDeviceInner>>,
}

impl DeviceClient for FakeClient {
    fn pull_attendance(&mut self) -> Result<Vec<RawAttendance>, DeviceError> {
        let inner = self.inner.lock().expect("fake lock");
        if let Some(error) = &inner.pull_error {
            return Err(DeviceError::Message(error.clone()));
        }
        Ok(inner.attendance.clone())
    }

    fn clear_attendance(&mut self) -> Result<(), DeviceError> {
        let mut inner = self.inner.lock().expect("fake lock");
        if let Some(error) = &inner.clear_error {
            return Err(DeviceError::Message(error.clone()));
        }
        inner.cleared = true;
        Ok(())
    }

    fn get_templates(&mut self) -> Result<Vec<FingerTemplate>, DeviceError> {
        Ok(Vec::new())
    }

    fn push_user_template(
        &mut self,
        _user: &DeviceUser,
        _finger: &FingerTemplate,
    ) -> Result<(), DeviceError> {
        Ok(())
    }

    fn disconnect(&mut self) {
        self.inner.lock().expect("fake lock").disconnected = true;
    }
}

#[derive(Debug)]
struct FakeHrms {
    fail: bool,
    org_timezone: Option<String>,
    received_batches: Mutex<Vec<Vec<HrmsEvent>>>,
}

impl FakeHrms {
    fn ok() -> Self {
        Self {
            fail: false,
            org_timezone: None,
            received_batches: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            fail: true,
            org_timezone: None,
            received_batches: Mutex::new(Vec::new()),
        }
    }

    fn with_org_timezone(timezone: &str) -> Self {
        Self {
            fail: false,
            org_timezone: Some(timezone.to_string()),
            received_batches: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.received_batches.lock().expect("hrms lock").len()
    }

    fn last_batch(&self) -> Vec<HrmsEvent> {
        self.received_batches
            .lock()
            .expect("hrms lock")
            .last()
            .cloned()
            .unwrap_or_default()
    }
}

impl HrmsClient for FakeHrms {
    fn forward_events(
        &self,
        _webhook_url: &str,
        _device: &BridgeDeviceConfig,
        events: &[HrmsEvent],
    ) -> Result<WebhookResult, HrmsError> {
        if self.fail {
            return Err(HrmsError::Message(
                "webhook down for DEVICE-1 with secret-key".to_string(),
            ));
        }
        self.received_batches
            .lock()
            .expect("hrms lock")
            .push(events.to_vec());
        Ok(WebhookResult {
            received: events.len(),
            org_timezone: self.org_timezone.clone(),
        })
    }
}

fn attendance(user_id: &str, timestamp: &str, punch: i64) -> RawAttendance {
    RawAttendance {
        user_id: user_id.to_string(),
        timestamp: timestamp.to_string(),
        punch,
    }
}

#[test]
fn happy_path_forwards_events_and_does_not_clear_by_default() {
    let connector = Arc::new(FakeConnector::with_attendance(vec![attendance(
        "7",
        "2026-05-21T10:05:00",
        0,
    )]));
    let hrms = Arc::new(FakeHrms::ok());
    let state = DeviceSyncState::new(
        device_config(false),
        "https://example.test/webhook".to_string(),
        connector.clone(),
        hrms.clone(),
    );

    let result = state.sync_once();

    assert!(result.ok);
    assert_eq!(result.pulled, 1);
    assert_eq!(result.forwarded, 1);
    assert!(!result.device_attendance_cleared);
    assert!(!connector.cleared());
    assert!(connector.disconnected());
    assert_eq!(hrms.call_count(), 1);
}

#[test]
fn clear_runs_after_success_when_configured() {
    let connector = Arc::new(FakeConnector::with_attendance(vec![attendance(
        "7",
        "2026-05-21T10:05:00",
        0,
    )]));
    let state = DeviceSyncState::new(
        device_config(true),
        "https://example.test/webhook".to_string(),
        connector.clone(),
        Arc::new(FakeHrms::ok()),
    );

    let result = state.sync_once();

    assert!(result.ok);
    assert!(result.device_attendance_cleared);
    assert!(connector.cleared());
}

#[test]
fn empty_attendance_does_not_call_hrms_or_clear() {
    let connector = Arc::new(FakeConnector::with_attendance(Vec::new()));
    let hrms = Arc::new(FakeHrms::ok());
    let state = DeviceSyncState::new(
        device_config(true),
        "https://example.test/webhook".to_string(),
        connector.clone(),
        hrms.clone(),
    );

    let result = state.sync_once();

    assert!(result.ok);
    assert_eq!(result.forwarded, 0);
    assert_eq!(hrms.call_count(), 0);
    assert!(!connector.cleared());
}

#[test]
fn webhook_failure_does_not_clear_attendance_and_redacts_secrets() {
    let connector = Arc::new(FakeConnector::with_attendance(vec![attendance(
        "7",
        "2026-05-21T10:05:00",
        0,
    )]));
    let state = DeviceSyncState::new(
        device_config(true),
        "https://example.test/webhook".to_string(),
        connector.clone(),
        Arc::new(FakeHrms::failing()),
    );

    let result = state.sync_once();

    assert!(!result.ok);
    assert!(!connector.cleared());
    assert!(!result.message.contains("secret-key"));
    assert!(!result.message.contains("DEVICE-1"));
    assert!(result.message.contains("***"));
}

#[test]
fn clear_failure_does_not_fail_successful_upload() {
    let connector = Arc::new(FakeConnector::with_attendance(vec![attendance(
        "7",
        "2026-05-21T10:05:00",
        0,
    )]));
    connector.fail_clear("device busy");
    let state = DeviceSyncState::new(
        device_config(true),
        "https://example.test/webhook".to_string(),
        connector.clone(),
        Arc::new(FakeHrms::ok()),
    );

    let result = state.sync_once();

    assert!(result.ok);
    assert!(!result.device_attendance_cleared);
    assert!(result.message.contains("failed to clear"));
}

#[test]
fn overlapping_sync_is_rejected() {
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let (started_tx, started_rx) = mpsc::channel();
    let connector = Arc::new(BlockingConnector {
        release: Arc::clone(&release),
        started: Mutex::new(Some(started_tx)),
    });
    let state = Arc::new(DeviceSyncState::new(
        device_config(false),
        "https://example.test/webhook".to_string(),
        connector,
        Arc::new(FakeHrms::ok()),
    ));

    let first_state = Arc::clone(&state);
    let thread = thread::spawn(move || first_state.sync_once());
    started_rx.recv().expect("first sync entered connector");

    let second = state.sync_once();

    {
        let (lock, cv) = &*release;
        *lock.lock().expect("release lock") = true;
        cv.notify_all();
    }
    let first = thread.join().expect("first sync finishes");

    assert!(first.ok);
    assert!(!second.ok);
    assert!(second.message.contains("in progress"));
}

/// A device_code unique to this test module run, distinct from `device_config`'s
/// "DEVICE-1", so persisted state files (last-result, org-timezone) don't
/// collide with other tests that share that code.
fn org_timezone_fallback_device_config() -> BridgeDeviceConfig {
    let mut cfg = device_config(false);
    cfg.device_code = "ORG-TZ-FALLBACK-TEST-DEVICE".to_string();
    cfg
}

#[test]
fn device_with_no_configured_timezone_learns_org_timezone_from_hrms_and_uses_it_next_sync() {
    // Ensure a cold start: persisted state from a previous run of this same
    // test (same device_code, same on-disk path) must not leak in and make
    // the "first sync" assertion below spuriously pass/fail.
    let cfg = org_timezone_fallback_device_config();
    let _ = std::fs::remove_file(fingerbridge::support::paths::device_last_result_path(
        &cfg.device_code,
    ));
    let _ = std::fs::remove_file(fingerbridge::support::paths::device_org_timezone_path(
        &cfg.device_code,
    ));

    // First sync: no deviceTimezone configured and HRMS hasn't told us the org
    // timezone yet, so the naive device timestamp is (wrongly, but
    // unavoidably on a cold start) treated as UTC.
    let connector = Arc::new(FakeConnector::with_attendance(vec![attendance(
        "7",
        "2026-07-07T15:51:07",
        0,
    )]));
    let hrms = Arc::new(FakeHrms::with_org_timezone("Asia/Dhaka"));
    let state = DeviceSyncState::new(
        org_timezone_fallback_device_config(),
        "https://example.test/webhook".to_string(),
        connector.clone(),
        hrms.clone(),
    );

    let first = state.sync_once();
    assert!(first.ok);
    let first_timestamp = hrms.last_batch()[0].timestamp.clone();
    assert_eq!(first_timestamp, "2026-07-07T15:51:07+00:00");

    // Second sync: the org timezone learned from the first response
    // (Asia/Dhaka, UTC+6) is now applied, converting the naive device
    // timestamp correctly instead of leaving it mislabeled as UTC.
    let connector2 = Arc::new(FakeConnector::with_attendance(vec![attendance(
        "7",
        "2026-07-07T16:00:00",
        0,
    )]));
    let state2 = DeviceSyncState::new(
        org_timezone_fallback_device_config(),
        "https://example.test/webhook".to_string(),
        connector2,
        hrms.clone(),
    );
    let second = state2.sync_once();
    assert!(second.ok);
    let second_timestamp = hrms.last_batch()[0].timestamp.clone();
    assert_eq!(second_timestamp, "2026-07-07T16:00:00+06:00");
}

struct BlockingConnector {
    release: Arc<(Mutex<bool>, Condvar)>,
    started: Mutex<Option<mpsc::Sender<()>>>,
}

impl DeviceConnector for BlockingConnector {
    fn connect(&self, _cfg: &BridgeDeviceConfig) -> Result<Box<dyn DeviceClient>, DeviceError> {
        if let Some(sender) = self.started.lock().expect("started lock").take() {
            sender.send(()).expect("send started signal");
        }

        let (lock, cv) = &*self.release;
        let mut released = lock.lock().expect("release lock");
        while !*released {
            released = cv.wait(released).expect("wait release");
        }

        Ok(Box::new(FakeClient {
            inner: Arc::new(Mutex::new(FakeDeviceInner::default())),
        }))
    }
}
