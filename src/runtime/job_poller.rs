//! HRMS job poller.
//!
//! The poller keeps the bridge outbound-only: it asks HRMS for pending jobs,
//! executes supported local actions, then reports completion back to HRMS.

use std::{collections::HashMap, thread, time::Duration};

use crate::{
    adapters::{
        device_zkteco_tcp::ZktecoTcpConnector,
        hrms_reqwest::{JobCompletion, PendingJob, ReqwestHrmsClient},
    },
    config::{BridgeConfig, BridgeDeviceConfig},
    domain::FingerTemplatePayload,
    ports::device::DeviceConnector,
    support::log,
};

/// Start a background HRMS job poller when job polling is configured.
pub fn start_job_poller(cfg: &BridgeConfig) {
    let Some(base_url) = cfg.hrms_base_url.clone() else {
        return;
    };
    let Some(token) = job_poll_token(cfg) else {
        log::warn(
            "jobpoll",
            format_args!("not started: no device apiKey available"),
        );
        return;
    };
    let interval = cfg.job_poll_interval_seconds;
    let device_codes = cfg
        .devices
        .iter()
        .map(|device| device.device_code.clone())
        .collect::<Vec<_>>();
    let devices = cfg
        .devices
        .iter()
        .cloned()
        .map(|device| (device.device_code.clone(), device))
        .collect::<HashMap<_, _>>();

    thread::spawn(move || {
        let client = ReqwestHrmsClient::default();
        log::info(
            "jobpoll",
            format_args!(
                "started (baseUrl={base_url}, every {interval}s, devices=[{}])",
                device_codes.join(", ")
            ),
        );
        loop {
            thread::sleep(Duration::from_secs(interval));
            match poll_once(&client, &base_url, &token, &device_codes, &devices) {
                Ok(0) => {} // nothing pending — stay quiet
                Ok(n) => log::info("jobpoll", format_args!("processed {n} job(s)")),
                Err(err) => log::error("jobpoll", format_args!("cycle failed: {err}")),
            }
        }
    });
}

fn poll_once(
    client: &ReqwestHrmsClient,
    base_url: &str,
    token: &str,
    device_codes: &[String],
    devices: &HashMap<String, BridgeDeviceConfig>,
) -> Result<usize, String> {
    let jobs = client
        .fetch_pending_jobs(base_url, token, device_codes)
        .map_err(|err| err.to_string())?;
    let count = jobs.len();
    if count > 0 {
        log::info("jobpoll", format_args!("{count} pending job(s)"));
    }
    let connector = ZktecoTcpConnector;
    for job in jobs {
        log::info(
            "jobpoll",
            format_args!("→ {} {} (job {})", job.job_type, job.device_code, job.id),
        );
        let completion = execute_job(&job, devices, &connector);
        let outcome = if completion.ok {
            "ok".to_string()
        } else {
            format!(
                "FAILED: {}",
                completion.error.as_deref().unwrap_or("unknown error")
            )
        };
        if let Err(err) = client.complete_job(base_url, token, &job.id, &completion) {
            log::error(
                "jobpoll",
                format_args!("✗ could not report job {} completion: {err}", job.id),
            );
        } else {
            let level = if completion.ok {
                log::Level::Info
            } else {
                log::Level::Error
            };
            log::event(
                level,
                "jobpoll",
                format_args!("✓ {} {} → {outcome}", job.job_type, job.device_code),
            );
        }
    }
    Ok(count)
}

fn execute_job(
    job: &PendingJob,
    devices: &HashMap<String, BridgeDeviceConfig>,
    connector: &dyn DeviceConnector,
) -> JobCompletion {
    let Some(device) = devices.get(&job.device_code) else {
        return JobCompletion::error(format!(
            "device '{}' is not configured on this bridge",
            job.device_code
        ));
    };

    match job.job_type.as_str() {
        "PULL_TEMPLATES" => pull_templates(device, connector),
        "PUSH_USER" => push_user(job, device, connector),
        other => JobCompletion::error(format!("unknown job type '{other}'")),
    }
}

fn pull_templates(device: &BridgeDeviceConfig, connector: &dyn DeviceConnector) -> JobCompletion {
    let mut client = match connector.connect(device) {
        Ok(client) => client,
        Err(err) => return JobCompletion::error(err.to_string()),
    };
    let result = match client.get_templates() {
        Ok(templates) => JobCompletion::templates(
            templates
                .iter()
                .map(|template| template.to_payload())
                .collect(),
        ),
        Err(err) => JobCompletion::error(err.to_string()),
    };
    client.disconnect();
    result
}

fn push_user(
    job: &PendingJob,
    device: &BridgeDeviceConfig,
    connector: &dyn DeviceConnector,
) -> JobCompletion {
    let Some(payload) = job.payload.clone() else {
        return JobCompletion::error("PUSH_USER payload is required");
    };
    let payload: FingerTemplatePayload = match serde_json::from_value(payload) {
        Ok(payload) => payload,
        Err(err) => return JobCompletion::error(format!("PUSH_USER payload is invalid: {err}")),
    };
    let user = payload.user();
    let finger = match payload.decode() {
        Ok(finger) => finger,
        Err(err) => return JobCompletion::error(err),
    };

    let mut client = match connector.connect(device) {
        Ok(client) => client,
        Err(err) => return JobCompletion::error(err.to_string()),
    };
    let result = match client.push_user_template(&user, &finger) {
        Ok(()) => JobCompletion::ok(),
        Err(err) => JobCompletion::error(err.to_string()),
    };
    client.disconnect();
    result
}

fn job_poll_token(cfg: &BridgeConfig) -> Option<String> {
    cfg.hrms_api_token
        .clone()
        .or_else(|| cfg.devices.first().map(|device| device.api_key.clone()))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use serde_json::json;

    use crate::{
        adapters::hrms_reqwest::PendingJob,
        config::BridgeDeviceConfig,
        domain::{DeviceUser, FingerTemplate, RawAttendance},
        ports::device::{DeviceClient, DeviceConnector, DeviceError},
    };

    use super::{execute_job, job_poll_token};

    fn test_device() -> BridgeDeviceConfig {
        BridgeDeviceConfig {
            device_ip: "127.0.0.1".to_string(),
            device_port: 4370,
            device_password: 0,
            device_timeout: 15,
            device_force_udp: false,
            device_omit_ping: true,
            device_timezone: None,
            device_code: "GATE-01".to_string(),
            api_key: "device-key".to_string(),
            organization_id: 1,
            sync_interval_seconds: 300,
            clear_attendance_after_sync: false,
            clear_attendance_threshold: None,
        }
    }

    fn devices() -> HashMap<String, BridgeDeviceConfig> {
        HashMap::from([("GATE-01".to_string(), test_device())])
    }

    #[test]
    fn unknown_device_job_returns_failure_completion() {
        let completion = execute_job(
            &PendingJob {
                id: "job-1".to_string(),
                job_type: "PUSH_USER".to_string(),
                device_code: "GHOST".to_string(),
                payload: None,
            },
            &devices(),
            &FakeConnector::default(),
        );
        assert!(!completion.ok);
        assert!(completion.error.expect("error").contains("not configured"));
    }

    #[test]
    fn pull_templates_returns_templates_from_device() {
        let completion = execute_job(
            &PendingJob {
                id: "job-1".to_string(),
                job_type: "PULL_TEMPLATES".to_string(),
                device_code: "GATE-01".to_string(),
                payload: Some(json!(null)),
            },
            &devices(),
            &FakeConnector::with_templates(vec![FingerTemplate {
                uid: 1,
                fid: 0,
                user_id: "001".to_string(),
                name: "Alice".to_string(),
                template: vec![1, 2, 3],
            }]),
        );
        assert!(completion.ok);
        assert_eq!(completion.templates.expect("templates").len(), 1);
    }

    #[test]
    fn unknown_job_type_returns_failure_completion() {
        let completion = execute_job(
            &PendingJob {
                id: "job-1".to_string(),
                job_type: "NOPE".to_string(),
                device_code: "GATE-01".to_string(),
                payload: None,
            },
            &devices(),
            &FakeConnector::default(),
        );
        assert!(!completion.ok);
        assert!(completion
            .error
            .expect("error")
            .contains("unknown job type"));
    }

    #[test]
    fn token_prefers_explicit_hrms_api_token() {
        let cfg = crate::config::BridgeConfig::from_json_value(json!({
            "vpsWebhookUrl": "https://example.test/webhook",
            "hrmsBaseUrl": "https://example.test/api/v1",
            "hrmsApiToken": "explicit",
            "devices": [{
                "deviceIp": "127.0.0.1",
                "deviceCode": "GATE-01",
                "apiKey": "device-key"
            }]
        }))
        .expect("config");
        assert_eq!(job_poll_token(&cfg).expect("token"), "explicit");
    }

    #[test]
    fn push_user_validates_payload_and_calls_device() {
        let connector = FakeConnector::default();
        let completion = execute_job(
            &PendingJob {
                id: "job-1".to_string(),
                job_type: "PUSH_USER".to_string(),
                device_code: "GATE-01".to_string(),
                payload: Some(json!({
                    "uid": 1,
                    "fid": 0,
                    "userId": "001",
                    "name": "Alice",
                    "templateBytes": "AQID"
                })),
            },
            &devices(),
            &connector,
        );
        assert!(completion.ok);
        assert_eq!(*connector.pushes.lock().expect("pushes"), 1);
    }

    #[derive(Clone, Default)]
    struct FakeConnector {
        templates: Vec<FingerTemplate>,
        pushes: Arc<Mutex<usize>>,
    }

    impl FakeConnector {
        fn with_templates(templates: Vec<FingerTemplate>) -> Self {
            Self {
                templates,
                pushes: Arc::new(Mutex::new(0)),
            }
        }
    }

    impl DeviceConnector for FakeConnector {
        fn connect(&self, _cfg: &BridgeDeviceConfig) -> Result<Box<dyn DeviceClient>, DeviceError> {
            Ok(Box::new(FakeClient {
                templates: self.templates.clone(),
                pushes: Arc::clone(&self.pushes),
            }))
        }
    }

    struct FakeClient {
        templates: Vec<FingerTemplate>,
        pushes: Arc<Mutex<usize>>,
    }

    impl DeviceClient for FakeClient {
        fn pull_attendance(&mut self) -> Result<Vec<RawAttendance>, DeviceError> {
            Ok(Vec::new())
        }

        fn clear_attendance(&mut self) -> Result<(), DeviceError> {
            Ok(())
        }

        fn get_templates(&mut self) -> Result<Vec<FingerTemplate>, DeviceError> {
            Ok(self.templates.clone())
        }

        fn push_user_template(
            &mut self,
            _user: &DeviceUser,
            _finger: &FingerTemplate,
        ) -> Result<(), DeviceError> {
            *self.pushes.lock().expect("pushes") += 1;
            Ok(())
        }

        fn disconnect(&mut self) {}
    }
}
