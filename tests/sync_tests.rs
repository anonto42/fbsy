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
    received_batches: Mutex<Vec<Vec<HrmsEvent>>>,
}

impl FakeHrms {
    fn ok() -> Self {
        Self {
            fail: false,
            received_batches: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            fail: true,
            received_batches: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.received_batches.lock().expect("hrms lock").len()
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
